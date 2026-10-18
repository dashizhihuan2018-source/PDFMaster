import Foundation
import PDFKit
import AppKit
import CoreText
import Vision
import Darwin

public enum OrganizeError: Error, Equatable { case unavailable, invalidInput, encrypted, signed, complexAssembly, unsupportedGeometry, cancelled, outputExists, processFailed(String) }
public struct NumberOptions {
    public var pages: [Int] = [] // zero-based, ascending; empty = all
    public var start = 1
    public var digits = 0
    public var prefix = ""
    public var suffix = ""
    public var top = false
    public var fontSize: CGFloat = 12
    public var margin: CGFloat = 24
    public init() {}
}
public struct PageInput { public let url: URL; public let pages: [Int]; public init(_ url: URL, pages: [Int] = []) { self.url = url; self.pages = pages } }
public struct PageLabel {
    public enum Style: String { case decimal = "D", roman = "r", upperRoman = "R", cover = "" }
    public var first: Int; public var style: Style; public var start: Int; public var prefix: String
    public init(first: Int, style: Style, start: Int = 1, prefix: String = "") { self.first = first; self.style = style; self.start = start; self.prefix = prefix }
}
public enum PDFCompressionPreset: CaseIterable {
    case lossless, highQuality, balanced, smallSize
    public var title: String {
        switch self {
        case .lossless: return "无损优化"
        case .highQuality: return "高清晰度"
        case .balanced: return "均衡压缩"
        case .smallSize: return "更小文件"
        }
    }
    public var jpegQuality: Int? {
        switch self {
        case .lossless: return nil
        case .highQuality: return 85
        case .balanced: return 65
        case .smallSize: return 40
        }
    }
    /// Target sampling density, separate from the JPEG encoder quality.
    public var imageDPI: Int? {
        switch self {
        case .lossless: return nil
        case .highQuality: return 300
        case .balanced: return 150
        case .smallSize: return 72
        }
    }
}
public struct CompressionResult { public let before: Int; public let after: Int; public var reduced: Bool { after < before } }
public struct OCRPage { public let page: Int; public let lines: [String]; public let skipped: Bool }
public struct PageDifference { public let page: Int; public let textChanged: Bool; public let changedPixelFraction: Double; public let missingSide: String? }
public final class OrganizeEngine {
    public let qpdf: URL; public let scratch: URL
    public init(qpdf: URL, scratch: URL) { self.qpdf = qpdf; self.scratch = scratch }
    public func number(_ input: URL, output: URL, options: NumberOptions = .init(), cancelled: () -> Bool = { false }) throws {
        try checkCancellation(cancelled); try ensureNew(output)
        let doc = try document(input)
        let pages = options.pages.isEmpty ? Array(0..<doc.pageCount) : options.pages
        guard !pages.isEmpty, pages == Array(Set(pages)).sorted(), pages.allSatisfy({ $0 >= 0 && $0 < doc.pageCount }),
              options.start >= 0, !options.start.addingReportingOverflow(pages.count - 1).overflow,
              (0...18).contains(options.digits), options.fontSize.isFinite, (4...144).contains(options.fontSize),
              options.margin.isFinite, options.margin >= 0, options.prefix.count + options.suffix.count <= 200,
              !(options.prefix + options.suffix).unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else { throw OrganizeError.invalidInput }
        let job = try jobDirectory(); defer { try? FileManager.default.removeItem(at: job) }
        _ = try inspect(input, job: job, assembly: false, cancelled: cancelled)
        let overlay = job.appendingPathComponent("numbers.pdf")
        try makeOverlay(doc, output: overlay, cancelled: cancelled) { index, box, ctx in
            guard let ordinal = pages.firstIndex(of: index) else { return }
            let value = String(options.start + ordinal)
            let label = options.prefix + String(repeating: "0", count: max(0, options.digits - value.count)) + value + options.suffix
            let line = CTLineCreateWithAttributedString(NSAttributedString(string: label, attributes: [.font: NSFont.systemFont(ofSize: options.fontSize), .foregroundColor: NSColor.black]))
            var ascent: CGFloat = 0, descent: CGFloat = 0
            let width = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, nil))
            guard width + 2 * options.margin <= box.width, ascent + descent + 2 * options.margin <= box.height else { throw OrganizeError.invalidInput }
            ctx.textPosition = CGPoint(x: (box.width - width)/2, y: options.top ? box.height - options.margin - ascent : options.margin + descent)
            CTLineDraw(line, ctx)
        }
        try overlayAndPublish(input, overlay: overlay, output: output, job: job, cancelled: cancelled)
    }

    public func labels(_ input: URL, output: URL, labels: [PageLabel], cancelled: () -> Bool = { false }) throws {
        try checkCancellation(cancelled); try ensureNew(output); let doc = try document(input)
        let indexes = labels.map(\.first)
        guard indexes.first == 0, indexes == Array(Set(indexes)).sorted(), indexes.allSatisfy({ $0 >= 0 && $0 < doc.pageCount }),
              labels.allSatisfy({ $0.start >= 1 && $0.prefix.count <= 200 && !$0.prefix.contains("\u{0}") }) else { throw OrganizeError.invalidInput }
        let job = try jobDirectory(); defer { try? FileManager.default.removeItem(at: job) }
        _ = try inspect(input, job: job, assembly: false, cancelled: cancelled)
        let candidate = job.appendingPathComponent("labels.pdf")
        let specs = labels.map { "\($0.first + 1):\($0.style.rawValue)/\($0.start)/\($0.prefix)" }
        _ = try run([input.path, "--set-page-labels"] + specs + ["--", candidate.path], job: job, cancelled: cancelled)
        try publish(candidate, to: output, job: job, cancelled: cancelled)
    }

    /// Ordered page assembly. Empty selection means all. Complex catalogs are rejected,
    /// rather than silently losing cross-document destinations, outlines, forms or tagging.
    public func assemble(_ inputs: [PageInput], output: URL, rotation: Int = 0, cancelled: () -> Bool = { false }) throws {
        try checkCancellation(cancelled); try ensureNew(output)
        guard !inputs.isEmpty, inputs.count <= 100, [0,90,180,270].contains(rotation) else { throw OrganizeError.invalidInput }
        let job = try jobDirectory(); defer { try? FileManager.default.removeItem(at: job) }
        var args = [inputs[0].url.path, "--pages"]
        var count = 0
        for input in inputs {
            try checkCancellation(cancelled)
            let doc = try document(input.url)
            _ = try inspect(input.url, job: job, assembly: true, cancelled: cancelled)
            let pages = input.pages.isEmpty ? Array(0..<doc.pageCount) : input.pages
            guard !pages.isEmpty, pages.allSatisfy({ $0 >= 0 && $0 < doc.pageCount }), count + pages.count <= 10000 else { throw OrganizeError.invalidInput }
            count += pages.count
            args += [input.url.path, pages.map { String($0 + 1) }.joined(separator: ",")]
        }
        let candidate = job.appendingPathComponent("assembled.pdf")
        args += ["--"]
        if rotation != 0 { args += ["--rotate=+\(rotation):1-z"] }
        args += [candidate.path]
        _ = try run(args, job: job, cancelled: cancelled)
        guard try document(candidate).pageCount == count else { throw OrganizeError.invalidInput }
        try publish(candidate, to: output, job: job, cancelled: cancelled)
    }

    /// Checks the admitted source bytes before any PDFKit serialization can discard signature fields.
    public func validateCompressionInput(_ input: URL, cancelled: () -> Bool = { false }) throws {
        try checkCancellation(cancelled); _ = try document(input)
        let job = try jobDirectory(); defer { try? FileManager.default.removeItem(at: job) }
        _ = try inspect(input, job: job, assembly: false, cancelled: cancelled)
    }

    /// Optimizes PDF objects and, when requested, eligible images without rasterizing pages.
    /// If the result grows, the unchanged source is published instead.
    public func compress(_ input: URL, output: URL, preset: PDFCompressionPreset = .lossless, cancelled: () -> Bool = { false }) throws -> CompressionResult {
        try checkCancellation(cancelled); try ensureNew(output); _ = try document(input)
        let job = try jobDirectory(); defer { try? FileManager.default.removeItem(at: job) }
        let inspection = try inspect(input, job: job, assembly: false, cancelled: cancelled)
        let candidate = job.appendingPathComponent("compact.pdf")
        var arguments = [input.path, "--object-streams=generate", "--compress-streams=y", "--recompress-flate", "--compression-level=9"]
        if let dpi = preset.imageDPI, let quality = preset.jpegQuality,
           let update = try PDFImageCompression.prepare(input: input, inspection: inspection, job: job,
                dpi: dpi, quality: quality, cancelled: cancelled,
                run: { args, destination, limit in
                    try self.run(args, job: job, cancelled: cancelled, stdout: destination, byteLimit: limit)
                }) {
            // Images have already been encoded once. Do not run --optimize-images again.
            arguments += ["--update-from-json=\(update.path)"]
        }
        _ = try run(arguments + [candidate.path], job: job, cancelled: cancelled)
        let before = try fileSize(input), optimized = try fileSize(candidate)
        try publish(optimized < before ? candidate : input, to: output, job: job, cancelled: cancelled)
        return CompressionResult(before: before, after: min(before, optimized))
    }

    /// Offline Vision OCR. Existing text pages are skipped. No scan image is painted into
    /// the overlay: all original page content, links and catalog stay in the base PDF.
    public func ocr(_ input: URL, output: URL, languages: [String] = ["zh-Hans", "en-US"], cancelled: () -> Bool = { false }) throws -> [OCRPage] {
        try checkCancellation(cancelled); try ensureNew(output); let doc = try document(input)
        guard !languages.isEmpty else { throw OrganizeError.invalidInput }
        let request = VNRecognizeTextRequest(); request.recognitionLevel = .accurate
        request.recognitionLanguages = languages; request.usesLanguageCorrection = false
        let supported = try request.supportedRecognitionLanguages()
        guard languages.allSatisfy(supported.contains) else { throw OrganizeError.invalidInput }
        let job = try jobDirectory(); defer { try? FileManager.default.removeItem(at: job) }
        _ = try inspect(input, job: job, assembly: false, cancelled: cancelled)
        var results: [OCRPage] = []
        let overlay = job.appendingPathComponent("ocr-layer.pdf")
        try makeOverlay(doc, output: overlay, cancelled: cancelled) { index, box, ctx in
            let page = doc.page(at: index)!
            if !(page.string ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                results.append(OCRPage(page: index, lines: [], skipped: true)); return
            }
            let image = try raster(page, scale: 2)
            try VNImageRequestHandler(cgImage: image, options: [:]).perform([request])
            try checkCancellation(cancelled)
            var lines: [String] = []
            for observation in request.results ?? [] {
                guard let candidate = observation.topCandidates(1).first, !candidate.string.isEmpty else { continue }
                let rect = observation.boundingBox
                // PDFKit/Poppler resolve this font's CJK mapping. CoreText omits
                // ToUnicode here, so pypdf text extraction remains a known limitation.
                guard let font = NSFont(name: "HiraginoSansGB-W3", size: 20) else { throw OrganizeError.unavailable }
                let line = CTLineCreateWithAttributedString(NSAttributedString(string: candidate.string, attributes: [.font: font]))
                var ascent: CGFloat = 0, descent: CGFloat = 0
                let width = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, nil))
                guard width > 0, ascent + descent > 0 else { continue }
                ctx.saveGState(); ctx.translateBy(x: rect.minX * box.width, y: rect.minY * box.height)
                ctx.scaleBy(x: rect.width * box.width / width, y: rect.height * box.height / (ascent + descent))
                ctx.textMatrix = .identity; ctx.textPosition = CGPoint(x: 0, y: descent); ctx.setTextDrawingMode(.invisible)
                CTLineDraw(line, ctx); ctx.restoreGState(); lines.append(candidate.string)
            }
            results.append(OCRPage(page: index, lines: lines, skipped: false))
        }
        if results.allSatisfy({ $0.lines.isEmpty }) { try publish(input, to: output, job: job, cancelled: cancelled) }
        else { try overlayAndPublish(input, overlay: overlay, output: output, job: job, cancelled: cancelled) }
        return results
    }

    /// Position-aligned comparison, not semantic page matching. Text and rendered pixels
    /// are separate signals; zero pixel difference at 72dpi is not full PDF equivalence.
    public func compare(_ left: URL, _ right: URL, cancelled: () -> Bool = { false }) throws -> [PageDifference] {
        try checkCancellation(cancelled); let a = try document(left), b = try document(right)
        var result: [PageDifference] = []
        for i in 0..<max(a.pageCount,b.pageCount) {
            try checkCancellation(cancelled)
            guard let p = a.page(at: i), let q = b.page(at: i) else {
                result.append(PageDifference(page: i, textChanged: true, changedPixelFraction: 1, missingSide: a.page(at: i) == nil ? "left" : "right")); continue
            }
            let pa = try raster(p, scale: 1), pb = try raster(q, scale: 1)
            var fraction = 1.0
            if pa.width == pb.width, pa.height == pb.height, let da = pa.dataProvider?.data, let db = pb.dataProvider?.data {
                let aa = CFDataGetBytePtr(da)!, bb = CFDataGetBytePtr(db)!
                var changed = 0
                for y in 0..<pa.height { for x in 0..<pa.width {
                    let ia = y * pa.bytesPerRow + x * 4, ib = y * pb.bytesPerRow + x * 4
                    if (0..<3).contains(where: { abs(Int(aa[ia+$0]) - Int(bb[ib+$0])) > 8 }) { changed += 1 }
                }}
                fraction = Double(changed) / Double(pa.width * pa.height)
            }
            result.append(PageDifference(page: i, textChanged: p.string != q.string, changedPixelFraction: fraction, missingSide: nil))
        }
        return result
    }

    private func checkCancellation(_ cancelled: () -> Bool) throws { if cancelled() { throw OrganizeError.cancelled } }
    private func ensureNew(_ url: URL) throws {
        guard url.isFileURL else { throw OrganizeError.invalidInput }
        if (try? FileManager.default.attributesOfItem(atPath: url.path)) != nil { throw OrganizeError.outputExists }
    }
    private func fileSize(_ url: URL) throws -> Int {
        let attributes = try FileManager.default.attributesOfItem(atPath: url.resolvingSymlinksInPath().path)
        guard attributes[.type] as? FileAttributeType == .typeRegular, let size = attributes[.size] as? NSNumber else { throw OrganizeError.invalidInput }
        return size.intValue
    }
    private func document(_ input: URL) throws -> PDFDocument {
        guard input.isFileURL, try fileSize(input) <= 256 * 1024 * 1024, let doc = PDFDocument(url: input) else { throw OrganizeError.invalidInput }
        guard !doc.isEncrypted, !doc.isLocked else { throw OrganizeError.encrypted }
        guard doc.pageCount > 0, doc.pageCount <= 10000 else { throw OrganizeError.invalidInput }
        return doc
    }
    private func jobDirectory() throws -> URL {
        guard scratch.isFileURL, qpdf.isFileURL else { throw OrganizeError.invalidInput }
        let dir = scratch.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        return dir
    }
    private func run(_ args: [String], job: URL, cancelled: () -> Bool = { false }, stdout: URL? = nil, byteLimit: Int = 32 * 1024 * 1024) throws -> Data {
        try checkCancellation(cancelled)
        let log = job.appendingPathComponent(UUID().uuidString + ".log")
        FileManager.default.createFile(atPath: log.path, contents: nil, attributes: [.posixPermissions: 0o600])
        let handle = try FileHandle(forWritingTo: log); defer { try? handle.close() }
        var outputHandle: FileHandle?
        if let stdout {
            guard FileManager.default.createFile(atPath: stdout.path, contents: nil, attributes: [.posixPermissions: 0o600]) else { throw OrganizeError.invalidInput }
            outputHandle = try FileHandle(forWritingTo: stdout)
        }
        defer { try? outputHandle?.close() }
        let process = Process(); process.executableURL = qpdf; process.arguments = args
        process.standardOutput = outputHandle ?? handle; process.standardError = handle
        try process.run(); let deadline = Date().addingTimeInterval(120)
        while process.isRunning {
            let logSize = (try? fileSize(log)) ?? 0
            let outputSize = stdout.flatMap { try? fileSize($0) } ?? 0
            if cancelled() || Date() > deadline || logSize > byteLimit || outputSize > byteLimit {
                process.terminate()
                let stop = Date().addingTimeInterval(1)
                while process.isRunning && Date() < stop { Thread.sleep(forTimeInterval: 0.01) }
                if process.isRunning { kill(process.processIdentifier, SIGKILL) }
                process.waitUntilExit()
                if cancelled() { throw OrganizeError.cancelled }
                throw OrganizeError.processFailed("qpdf timeout or output budget exceeded")
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
        process.waitUntilExit(); try handle.synchronize()
        try outputHandle?.synchronize()
        let outputSize = try stdout.map { try fileSize($0) } ?? 0
        guard try fileSize(log) <= byteLimit, outputSize <= byteLimit else {
            throw OrganizeError.processFailed("qpdf output budget exceeded")
        }
        let data = try Data(contentsOf: log)
        guard process.terminationStatus == 0 else { throw OrganizeError.processFailed(String(decoding: data.prefix(4000), as: UTF8.self)) }
        return data
    }
    private func inspect(_ input: URL, job: URL, assembly: Bool, cancelled: () -> Bool = { false }) throws -> [String: Any] {
        let data = try run(["--json", "--json-key=encrypt", "--json-key=qpdf", "--json-key=pages", "--json-stream-data=none", input.path], job: job, cancelled: cancelled)
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any], let encryption = json["encrypt"] as? [String: Any], let encrypted = encryption["encrypted"] as? Bool else { throw OrganizeError.invalidInput }
        if encrypted { throw OrganizeError.encrypted }
        func inspectValue(_ value: Any) throws {
            if let dict = value as? [String: Any] {
                if dict["/FT"] as? String == "/Sig" || dict["/Type"] as? String == "/Sig" || dict["/ByteRange"] != nil { throw OrganizeError.signed }
                if assembly {
                    // Allow only simple catalogs for now. Names, outlines, tags, forms and
                    // navigation actions need explicit cross-document remapping.
                    if dict["/Type"] as? String == "/Catalog" {
                        let allowed: Set<String> = ["/Type","/Pages","/Metadata","/ViewerPreferences","/PageMode","/PageLayout","/Lang","/Version","/PageLabels"]
                        if !Set(dict.keys).isSubset(of: allowed) { throw OrganizeError.complexAssembly }
                    }
                    if dict["/Dest"] != nil || dict["/S"] as? String == "/GoTo" { throw OrganizeError.complexAssembly }
                }
                for v in dict.values { try inspectValue(v) }
            } else if let array = value as? [Any] { for v in array { try inspectValue(v) } }
        }
        guard let objects = json["qpdf"] as? [Any], objects.count >= 2 else { throw OrganizeError.invalidInput }
        try inspectValue(objects); return json
    }
    private func publish(_ candidate: URL, to output: URL, job: URL, cancelled: () -> Bool = { false }) throws {
        try checkCancellation(cancelled); _ = try run(["--check",candidate.path], job: job, cancelled: cancelled)
        try ensureNew(output); try checkCancellation(cancelled)
        // Stage on the destination volume, then publish via exclusive hard-link.
        // A failed copy cannot leave a partial file at the requested output path.
        let staged = output.deletingLastPathComponent().appendingPathComponent(".pdf-organize-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: staged) }
        let resolved = candidate.resolvingSymlinksInPath()
        _ = try fileSize(resolved)
        try FileManager.default.copyItem(at: resolved, to: staged)
        guard try FileManager.default.attributesOfItem(atPath: staged.path)[.type] as? FileAttributeType == .typeRegular else { throw OrganizeError.invalidInput }
        try checkCancellation(cancelled)
        try FileManager.default.linkItem(at: staged, to: output)
    }
    private func overlayAndPublish(_ input: URL, overlay: URL, output: URL, job: URL, cancelled: () -> Bool) throws {
        let candidate = job.appendingPathComponent("result.pdf")
        _ = try run([input.path,"--overlay",overlay.path,"--",candidate.path], job: job, cancelled: cancelled)
        try publish(candidate, to: output, job: job, cancelled: cancelled)
    }
    private func makeOverlay(_ doc: PDFDocument, output: URL, cancelled: () -> Bool, draw: (Int, CGRect, CGContext) throws -> Void) throws {
        var first = doc.page(at: 0)!.bounds(for: .mediaBox)
        guard let ctx = CGContext(output as CFURL, mediaBox: &first, nil) else { throw OrganizeError.invalidInput }
        defer { ctx.closePDF() }
        for i in 0..<doc.pageCount {
            try checkCancellation(cancelled)
            let page = doc.page(at: i)!, box = page.bounds(for: .mediaBox)
            guard box.origin == .zero, box == page.bounds(for: .cropBox), page.rotation == 0,
                  box.width > 0, box.height > 0, box.width <= 14400, box.height <= 14400 else { throw OrganizeError.unsupportedGeometry }
            var media = box
            let data = NSData(bytes: &media, length: MemoryLayout<CGRect>.size)
            ctx.beginPDFPage([kCGPDFContextMediaBox: data] as CFDictionary)
            do { try draw(i,box,ctx); ctx.endPDFPage() }
            catch { ctx.endPDFPage(); throw error }
        }
    }
    private func raster(_ page: PDFPage, scale: CGFloat) throws -> CGImage {
        guard let reference = page.pageRef else { throw OrganizeError.invalidInput }
        let box = page.bounds(for: .cropBox)
        let rotated = abs(page.rotation % 180) == 90
        guard box.width.isFinite, box.height.isFinite, box.width > 0, box.height > 0,
              box.width * scale <= 16000, box.height * scale <= 16000 else { throw OrganizeError.unsupportedGeometry }
        let width = Int(ceil((rotated ? box.height : box.width) * scale)), height = Int(ceil((rotated ? box.width : box.height) * scale))
        guard width > 0, height > 0, width <= 16000, height <= 16000, width * height <= 24_000_000,
              let ctx = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { throw OrganizeError.unsupportedGeometry }
        ctx.setFillColor(NSColor.white.cgColor); ctx.fill(CGRect(x:0,y:0,width:width,height:height))
        // getDrawingTransform does not upscale a page on this SDK. Apply the
        // raster scale explicitly, then map crop/rotation in logical points.
        ctx.scaleBy(x: scale, y: scale)
        ctx.concatenate(reference.getDrawingTransform(.cropBox, rect: CGRect(x:0,y:0,width:CGFloat(width)/scale,height:CGFloat(height)/scale), rotate:0, preserveAspectRatio:true))
        ctx.drawPDFPage(reference)
        for annotation in page.annotations where annotation.shouldDisplay { annotation.draw(with: .cropBox, in: ctx) }
        guard let image = ctx.makeImage() else { throw OrganizeError.invalidInput }; return image
    }
}
