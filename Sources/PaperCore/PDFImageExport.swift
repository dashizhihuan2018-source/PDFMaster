import CoreGraphics
import Darwin
import Foundation
import ImageIO
import PDFKit
import UniformTypeIdentifiers

public enum PDFImageExportFormat: String, CaseIterable, Sendable {
    case png
    case jpeg

    public var fileExtension: String { self == .png ? "png" : "jpg" }
    fileprivate var contentType: UTType { self == .png ? .png : .jpeg }
}

public struct PDFImageExportRequest: Sendable {
    public var format: PDFImageExportFormat
    public var dpi: Double
    /// Zero-based page indexes. Files are emitted in ascending document order.
    public var pages: IndexSet
    public var jpegQuality: Double
    public var maximumPixelsPerPage: UInt64
    public var maximumTotalPixels: UInt64
    public var maximumOutputBytes: Int

    public init(
        format: PDFImageExportFormat,
        dpi: Double,
        pages: IndexSet,
        jpegQuality: Double = 0.9,
        maximumPixelsPerPage: UInt64 = 32_000_000,
        maximumTotalPixels: UInt64 = 100_000_000,
        maximumOutputBytes: Int = 512 * 1024 * 1024
    ) {
        self.format = format
        self.dpi = dpi
        self.pages = pages
        self.jpegQuality = jpegQuality
        self.maximumPixelsPerPage = maximumPixelsPerPage
        self.maximumTotalPixels = maximumTotalPixels
        self.maximumOutputBytes = maximumOutputBytes
    }
}

public struct PDFImageExportResult: Sendable, Equatable {
    public let directory: URL
    public let files: [URL]
    public let pageNumbers: [Int]
}

public enum PDFImageExportFailure: Error, Equatable, LocalizedError, Sendable {
    case invalidInput
    case encryptedInput
    case invalidSelection
    case invalidResolution
    case outputExists
    case tooLarge
    case renderFailed
    case writeFailed
    case cancelled

    public var errorDescription: String? {
        switch self {
        case .invalidInput: "无法读取此 PDF，或输出位置不是本机文件夹。"
        case .encryptedInput: "为保护权限，当前版本不导出加密来源 PDF。"
        case .invalidSelection: "页码范围无效。"
        case .invalidResolution: "分辨率须在 36–600 DPI 之间。"
        case .outputExists: "目标文件夹已存在；为避免覆盖，请选择新名称。"
        case .tooLarge: "所选页面在当前分辨率下超过本地安全预算。"
        case .renderFailed: "PDF 页面渲染失败。"
        case .writeFailed: "无法安全写入图片。"
        case .cancelled: "已取消导出；未发布不完整文件夹。"
        }
    }
}

public enum PDFImageExport {
    private static let inputByteLimit = 256 * 1024 * 1024
    private static let maximumDimension = 20_000

    /// Renders with native PDFKit/CoreGraphics and publishes the complete directory in one rename.
    /// The destination must not already exist. A failure or cancellation removes the private stage.
    public static func export(
        data: Data,
        to destination: URL,
        request: PDFImageExportRequest,
        cancelled: () -> Bool = { false }
    ) throws -> PDFImageExportResult {
        guard destination.isFileURL, data.count > 0, data.count <= inputByteLimit,
              let document = PDFDocument(data: data), document.pageCount > 0, document.pageCount <= 10_000 else {
            throw PDFImageExportFailure.invalidInput
        }
        guard !document.isEncrypted, !document.isLocked else { throw PDFImageExportFailure.encryptedInput }
        guard request.dpi.isFinite, (36...600).contains(request.dpi),
              request.jpegQuality.isFinite, (0...1).contains(request.jpegQuality) else {
            throw PDFImageExportFailure.invalidResolution
        }
        guard !request.pages.isEmpty,
              request.pages.allSatisfy({ (0..<document.pageCount).contains($0) }) else {
            throw PDFImageExportFailure.invalidSelection
        }
        let defaults = PDFImageExportRequest(format: .png, dpi: 72, pages: IndexSet(integer: 0))
        guard request.maximumPixelsPerPage > 0, request.maximumPixelsPerPage <= defaults.maximumPixelsPerPage,
              request.maximumTotalPixels > 0, request.maximumTotalPixels <= defaults.maximumTotalPixels,
              request.maximumOutputBytes > 0, request.maximumOutputBytes <= defaults.maximumOutputBytes else {
            throw PDFImageExportFailure.invalidInput
        }
        if cancelled() { throw PDFImageExportFailure.cancelled }

        let fileManager = FileManager.default
        let unresolvedParent = destination.deletingLastPathComponent()
        var parentIsDirectory: ObjCBool = false
        guard fileManager.fileExists(atPath: unresolvedParent.path, isDirectory: &parentIsDirectory),
              parentIsDirectory.boolValue else { throw PDFImageExportFailure.invalidInput }
        let parent = unresolvedParent.resolvingSymlinksInPath()
        let name = destination.lastPathComponent
        guard !name.isEmpty, name != ".", name != ".." else { throw PDFImageExportFailure.invalidInput }
        let target = parent.appendingPathComponent(name, isDirectory: true)
        var targetInfo = stat()
        guard lstat(target.path, &targetInfo) != 0, errno == ENOENT else { throw PDFImageExportFailure.outputExists }

        let stage = parent.appendingPathComponent(".pdf-image-export-\(UUID().uuidString)", isDirectory: true)
        guard Darwin.mkdir(stage.path, 0o700) == 0 else { throw PDFImageExportFailure.writeFailed }
        var stagePublished = false
        defer { if !stagePublished { try? fileManager.removeItem(at: stage) } }

        var generated: [URL] = []
        var totalPixels: UInt64 = 0
        var totalBytes = 0
        for pageIndex in request.pages {
            try autoreleasepool {
                if cancelled() { throw PDFImageExportFailure.cancelled }
                guard let page = document.page(at: pageIndex), let pageRef = page.pageRef else {
                    throw PDFImageExportFailure.invalidInput
                }
                let crop = pageRef.getBoxRect(.cropBox)
                let quarterTurns = ((pageRef.rotationAngle % 360) + 360) % 360
                let rotated = quarterTurns == 90 || quarterTurns == 270
                let pointWidth = rotated ? crop.height : crop.width
                let pointHeight = rotated ? crop.width : crop.height
                guard pointWidth.isFinite, pointHeight.isFinite, pointWidth > 0, pointHeight > 0 else {
                    throw PDFImageExportFailure.invalidInput
                }
                let scaledWidth = ceil(pointWidth * request.dpi / 72)
                let scaledHeight = ceil(pointHeight * request.dpi / 72)
                guard scaledWidth.isFinite, scaledHeight.isFinite,
                      scaledWidth > 0, scaledHeight > 0,
                      scaledWidth <= Double(maximumDimension), scaledHeight <= Double(maximumDimension) else {
                    throw PDFImageExportFailure.tooLarge
                }
                let width = Int(scaledWidth)
                let height = Int(scaledHeight)
                let pixels = UInt64(width).multipliedReportingOverflow(by: UInt64(height))
                guard !pixels.overflow, pixels.partialValue <= request.maximumPixelsPerPage else {
                    throw PDFImageExportFailure.tooLarge
                }
                let total = totalPixels.addingReportingOverflow(pixels.partialValue)
                guard !total.overflow, total.partialValue <= request.maximumTotalPixels else {
                    throw PDFImageExportFailure.tooLarge
                }
                totalPixels = total.partialValue

                guard let context = CGContext(
                    data: nil,
                    width: width,
                    height: height,
                    bitsPerComponent: 8,
                    bytesPerRow: 0,
                    space: CGColorSpaceCreateDeviceRGB(),
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
                ) else { throw PDFImageExportFailure.renderFailed }
                context.setFillColor(CGColor(gray: 1, alpha: 1))
                context.fill(CGRect(x: 0, y: 0, width: width, height: height))
                let outputRect = CGRect(x: 0, y: 0, width: width, height: height)
                context.concatenate(pageRef.getDrawingTransform(.cropBox, rect: outputRect, rotate: 0, preserveAspectRatio: true))
                context.drawPDFPage(pageRef)
                for annotation in page.annotations { annotation.draw(with: .cropBox, in: context) }
                guard let image = context.makeImage() else { throw PDFImageExportFailure.renderFailed }

                let fileName = String(format: "page-%04d.%@", pageIndex + 1, request.format.fileExtension)
                let file = stage.appendingPathComponent(fileName)
                guard let sink = CGImageDestinationCreateWithURL(file as CFURL, request.format.contentType.identifier as CFString, 1, nil) else {
                    throw PDFImageExportFailure.writeFailed
                }
                var properties: [CFString: Any] = [
                    kCGImagePropertyDPIWidth: request.dpi,
                    kCGImagePropertyDPIHeight: request.dpi
                ]
                if request.format == .jpeg { properties[kCGImageDestinationLossyCompressionQuality] = request.jpegQuality }
                CGImageDestinationAddImage(sink, image, properties as CFDictionary)
                guard CGImageDestinationFinalize(sink) else { throw PDFImageExportFailure.writeFailed }
                var fileInfo = stat()
                guard lstat(file.path, &fileInfo) == 0, (fileInfo.st_mode & S_IFMT) == S_IFREG,
                      fileInfo.st_size > 0, fileInfo.st_size <= request.maximumOutputBytes - totalBytes else {
                    throw PDFImageExportFailure.tooLarge
                }
                totalBytes += Int(fileInfo.st_size)
                generated.append(file)
                if cancelled() { throw PDFImageExportFailure.cancelled }
            }
        }

        guard generated.count == request.pages.count else { throw PDFImageExportFailure.writeFailed }
        guard renamex_np(stage.path, target.path, UInt32(RENAME_EXCL)) == 0 else {
            throw errno == EEXIST ? PDFImageExportFailure.outputExists : PDFImageExportFailure.writeFailed
        }
        stagePublished = true
        return PDFImageExportResult(
            directory: target,
            files: generated.map { target.appendingPathComponent($0.lastPathComponent) },
            pageNumbers: request.pages.map { $0 + 1 }
        )
    }
}
