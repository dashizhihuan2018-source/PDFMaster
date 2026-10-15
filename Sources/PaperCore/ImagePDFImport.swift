import Foundation
import CoreGraphics
import ImageIO
import Darwin
public enum ImportFailure: Error { case invalidInput, invalidLayout, outputExists, cancelled, decodeFailed, tooLarge, writeFailed }
public struct ImageLayout {
    public var size: CGSize?
    public var margin: Double
    public init(size: CGSize? = CGSize(width: 595.276, height: 841.89), margin: Double = 24) { self.size = size; self.margin = margin }
}
/// Per-job ceilings may be tightened by callers, never raised above the defaults.
/// Pixel budgets describe planned decoding, not a bound on ImageIO's internal memory.
public struct ImageImportLimits {
    public var maximumDecodedPixelsPerFrame: UInt64
    public var maximumTotalDecodedPixels: UInt64
    public var maximumPages: Int
    public var maximumOutputBytes: Int
    public init(maximumDecodedPixelsPerFrame: UInt64 = 40_000_000,
                maximumTotalDecodedPixels: UInt64 = 100_000_000,
                maximumPages: Int = 500, maximumOutputBytes: Int = 256 * 1024 * 1024) {
        self.maximumDecodedPixelsPerFrame = maximumDecodedPixelsPerFrame
        self.maximumTotalDecodedPixels = maximumTotalDecodedPixels
        self.maximumPages = maximumPages
        self.maximumOutputBytes = maximumOutputBytes
    }
}
public enum ImagePDFImport {
    static func readBounded(_ file: FileHandle, maximumBytes: Int, cancelled: () -> Bool = { false }) throws -> Data {
        guard maximumBytes >= 0 else { throw ImportFailure.tooLarge }
        var data = Data()
        while true {
            if cancelled() { throw ImportFailure.cancelled }
            let remaining = maximumBytes - data.count
            let chunk = try file.read(upToCount: min(64 * 1024, remaining + 1)) ?? Data()
            if chunk.isEmpty { return data }
            guard chunk.count <= remaining else { throw ImportFailure.tooLarge }
            data.append(chunk)
        }
    }
    private static func readImageData(_ input: URL, maximumBytes: Int, cancelled: () -> Bool) throws -> Data {
        guard input.isFileURL else { throw ImportFailure.invalidInput }
        let sourceFD = Darwin.open(input.path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK)
        guard sourceFD >= 0 else { throw ImportFailure.invalidInput }
        let file = FileHandle(fileDescriptor: sourceFD, closeOnDealloc: true)
        defer { try? file.close() }
        var info = stat()
        guard fstat(sourceFD, &info) == 0, (info.st_mode & S_IFMT) == S_IFREG, info.st_size > 0 else { throw ImportFailure.invalidInput }
        guard info.st_size <= maximumBytes else { throw ImportFailure.tooLarge }
        return try readBounded(file, maximumBytes: maximumBytes, cancelled: cancelled)
    }
    static func checkedPixelCount(width: Double, height: Double, maximum: UInt64) throws -> UInt64 {
        // Double(UInt64.max) rounds to 2^64: the exclusive comparison also keeps the
        // subsequent conversion from trapping at that rounded boundary.
        guard width.isFinite, height.isFinite, width > 0, height > 0,
              width.rounded(.towardZero) == width, height.rounded(.towardZero) == height,
              width < Double(UInt64.max), height < Double(UInt64.max) else { throw ImportFailure.tooLarge }
        let result = UInt64(width).multipliedReportingOverflow(by: UInt64(height))
        guard !result.overflow, result.partialValue <= maximum else { throw ImportFailure.tooLarge }
        return result.partialValue
    }
    static func checkedTotal(_ total: UInt64, adding pixels: UInt64, maximum: UInt64) throws -> UInt64 {
        let result = total.addingReportingOverflow(pixels)
        guard !result.overflow, result.partialValue <= maximum else { throw ImportFailure.tooLarge }
        return result.partialValue
    }
    private static func framePixels(_ source: CGImageSource, index: Int, maximum: UInt64) throws -> UInt64 {
        guard let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [CFString: Any],
              let width = (properties[kCGImagePropertyPixelWidth] as? NSNumber)?.doubleValue,
              let height = (properties[kCGImagePropertyPixelHeight] as? NSNumber)?.doubleValue else { throw ImportFailure.tooLarge }
        return try checkedPixelCount(width: width, height: height, maximum: maximum)
    }
    private static func checkOutputSize(_ stage: URL, maximum: Int) throws {
        var info = stat()
        guard lstat(stage.path, &info) == 0, (info.st_mode & S_IFMT) == S_IFREG, info.st_size >= 0 else { throw ImportFailure.writeFailed }
        guard info.st_size <= maximum else { throw ImportFailure.tooLarge }
    }
    private static func preflight(_ inputs: [URL], limits: ImageImportLimits, cancelled: () -> Bool) throws {
        var totalBytes = 0
        var pages = 0
        var totalPixels: UInt64 = 0
        for input in inputs {
            if cancelled() { throw ImportFailure.cancelled }
            let data = try readImageData(input, maximumBytes: min(128 * 1024 * 1024, 512 * 1024 * 1024 - totalBytes), cancelled: cancelled)
            totalBytes += data.count
            guard let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary) else { throw ImportFailure.decodeFailed }
            let count = CGImageSourceGetCount(source)
            guard count > 0, count <= limits.maximumPages - pages else { throw ImportFailure.tooLarge }
            let type = CGImageSourceGetType(source) as String? ?? ""
            guard count == 1 || type == "public.tiff" else { throw ImportFailure.invalidInput }
            for index in 0..<count {
                if cancelled() { throw ImportFailure.cancelled }
                let pixels = try framePixels(source, index: index, maximum: limits.maximumDecodedPixelsPerFrame)
                totalPixels = try checkedTotal(totalPixels, adding: pixels, maximum: limits.maximumTotalDecodedPixels)
            }
            pages += count
        }
    }
    /// Ordered files; every TIFF frame becomes a page. Animated GIF is rejected to avoid silent frame loss.
    /// Native size means one oriented pixel per PDF point, not inferred print DPI.
    /// All frame metadata is preflighted before creating a PDF stage; it is checked again
    /// immediately before decoding in case inputs change. Call off the UI thread.
    /// Cancellation is cooperative; it cannot interrupt one system ImageIO decode.
    /// Output bytes are checked after each page and after close, not by a hard-limited
    /// write stream: ImageIO/CGPDF can buffer or exceed the ceiling between checks.
    public static func convert(_ inputs: [URL], to output: URL, layout: ImageLayout = .init(), limits: ImageImportLimits = .init(), cancelled: () -> Bool = { false }) throws -> Int {
        guard !inputs.isEmpty, inputs.count <= 500, output.isFileURL, output.pathExtension.lowercased() == "pdf" else { throw ImportFailure.invalidInput }
        let defaults = ImageImportLimits()
        guard limits.maximumDecodedPixelsPerFrame > 0, limits.maximumDecodedPixelsPerFrame <= defaults.maximumDecodedPixelsPerFrame,
              limits.maximumTotalDecodedPixels > 0, limits.maximumTotalDecodedPixels <= defaults.maximumTotalDecodedPixels,
              limits.maximumPages > 0, limits.maximumPages <= defaults.maximumPages,
              limits.maximumOutputBytes > 0, limits.maximumOutputBytes <= defaults.maximumOutputBytes else { throw ImportFailure.invalidInput }
        guard layout.margin.isFinite, layout.margin >= 0 else { throw ImportFailure.invalidLayout }
        if let size = layout.size {
            guard size.width.isFinite, size.height.isFinite, size.width > layout.margin * 2, size.height > layout.margin * 2, size.width <= 14400, size.height <= 14400 else { throw ImportFailure.invalidLayout }
        }
        var statbuf = stat()
        guard lstat(output.path, &statbuf) != 0, errno == ENOENT else { throw ImportFailure.outputExists }
        if cancelled() { throw ImportFailure.cancelled }
        try preflight(inputs, limits: limits, cancelled: cancelled)
        if cancelled() { throw ImportFailure.cancelled }
        let parent = output.deletingLastPathComponent().resolvingSymlinksInPath()
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: parent.path, isDirectory: &isDirectory), isDirectory.boolValue else { throw ImportFailure.invalidInput }
        let target = parent.appendingPathComponent(output.lastPathComponent)
        let stage = parent.appendingPathComponent(".image-import-\(UUID().uuidString).pdf")
        let fd = Darwin.open(stage.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
        guard fd >= 0 else { throw ImportFailure.writeFailed }
        close(fd)
        defer { try? FileManager.default.removeItem(at: stage) }
        guard let consumer = CGDataConsumer(url: stage as CFURL), let context = CGContext(consumer: consumer, mediaBox: nil, [kCGPDFContextCreator: "PDF Master local image import"] as CFDictionary) else { throw ImportFailure.writeFailed }
        var closed = false
        defer { if !closed { context.closePDF() } }
        var pageCount = 0
        var totalBytes = 0
        var totalPixels: UInt64 = 0
        for input in inputs {
            if cancelled() { throw ImportFailure.cancelled }
            let budget = min(128 * 1024 * 1024, 512 * 1024 * 1024 - totalBytes)
            let data = try readImageData(input, maximumBytes: budget, cancelled: cancelled)
            totalBytes += data.count
            guard let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary) else { throw ImportFailure.decodeFailed }
            let count = CGImageSourceGetCount(source)
            guard count > 0, count <= limits.maximumPages - pageCount else { throw ImportFailure.tooLarge }
            let type = CGImageSourceGetType(source) as String? ?? ""
            guard count == 1 || type == "public.tiff" else { throw ImportFailure.invalidInput }
            for index in 0..<count {
                if cancelled() { throw ImportFailure.cancelled }
                let pixels = try framePixels(source, index: index, maximum: limits.maximumDecodedPixelsPerFrame)
                totalPixels = try checkedTotal(totalPixels, adding: pixels, maximum: limits.maximumTotalDecodedPixels)
                guard let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [CFString: Any],
                      let width = (properties[kCGImagePropertyPixelWidth] as? NSNumber)?.doubleValue,
                      let height = (properties[kCGImagePropertyPixelHeight] as? NSNumber)?.doubleValue else { throw ImportFailure.tooLarge }
                let options: [CFString: Any] = [kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceCreateThumbnailWithTransform: true, kCGImageSourceThumbnailMaxPixelSize: max(width, height), kCGImageSourceShouldCacheImmediately: true]
                guard let image = CGImageSourceCreateThumbnailAtIndex(source, index, options as CFDictionary) else { throw ImportFailure.decodeFailed }
                let imageSize = CGSize(width: image.width, height: image.height)
                let size = layout.size ?? imageSize
                let margin = layout.size == nil ? 0 : layout.margin
                guard size.width <= 14400, size.height <= 14400 else { throw ImportFailure.tooLarge }
                var box = CGRect(origin: .zero, size: size)
                let boxData = Data(bytes: &box, count: MemoryLayout<CGRect>.size)
                context.beginPDFPage([kCGPDFContextMediaBox: boxData] as CFDictionary)
                context.setFillColor(CGColor(gray: 1, alpha: 1)); context.fill(box)
                let scale = min((size.width - margin * 2) / imageSize.width, (size.height - margin * 2) / imageSize.height)
                let rect = CGRect(x: (size.width - imageSize.width * scale) / 2, y: (size.height - imageSize.height * scale) / 2, width: imageSize.width * scale, height: imageSize.height * scale)
                context.draw(image, in: rect); context.endPDFPage(); pageCount += 1
                try checkOutputSize(stage, maximum: limits.maximumOutputBytes)
            }
        }
        context.closePDF(); closed = true
        try checkOutputSize(stage, maximum: limits.maximumOutputBytes)
        guard let check = CGPDFDocument(stage as CFURL), check.numberOfPages == pageCount, pageCount > 0 else { throw ImportFailure.writeFailed }
        if cancelled() { throw ImportFailure.cancelled }
        // link(2) provides atomic create-if-absent; never follows or overwrites an existing target.
        guard link(stage.path, target.path) == 0 else { throw errno == EEXIST ? ImportFailure.outputExists : ImportFailure.writeFailed }
        return pageCount
    }
}
