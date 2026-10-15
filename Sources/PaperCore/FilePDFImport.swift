import Foundation
import PDFKit
import ImageIO
import Darwin

public enum FilePDFImportError: Error, LocalizedError {
    case unsupported(String), invalid(String), failed(String, String), tooLarge, cancelled
    public var errorDescription: String? {
        switch self {
        case .unsupported(let name): return "不支持 \(name)。可拖入 PDF、PNG/JPEG/TIFF/HEIC/BMP/静态 GIF、TXT/MD/RTF 或纯文字 DOCX；其他格式请先导出 PDF。整批未插入。"
        case .invalid(let name): return "无法读取 \(name)：只接受普通本地文件，不接受文件夹或符号链接。整批未插入。"
        case .failed(let name, let reason): return "\(name) 转换失败：\(reason)。整批未插入，原文件未修改。"
        case .tooLarge: return "文件批次超出安全上限（128 个文件、500 页、512 MB 输入或 256 MB 输出）。整批未插入。"
        case .cancelled: return "已取消导入，未插入任何页面。"
        }
    }
}

/// Convert on a worker queue, then publish the complete ordered batch in one
/// workspace transaction. Source descriptors are pinned and copied to an owned
/// directory: converters never write to the user's files or fetch network URLs.
public enum FilePDFImport {
    public static let imageExtensions = ["png", "jpg", "jpeg", "tif", "tiff", "heic", "heif", "bmp", "gif"]
    public static let textExtensions = ["txt", "md", "rtf", "docx"]
    public static func prepare(_ inputs: [URL], cancelled: () -> Bool = { false }) throws -> [Data] {
        guard !inputs.isEmpty, inputs.count <= 128 else { throw FilePDFImportError.tooLarge }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-files-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: directory) }
        var inputBytes = 0, outputBytes = 0, pages = 0
        var decodedPixels: UInt64 = 0
        var result: [Data] = []
        for (index, input) in inputs.enumerated() {
            if cancelled() { throw FilePDFImportError.cancelled }
            let ext = input.pathExtension.lowercased(), name = input.lastPathComponent
            guard ext == "pdf" || imageExtensions.contains(ext) || textExtensions.contains(ext) else { throw FilePDFImportError.unsupported(name) }
            let staged = directory.appendingPathComponent("source-\(index).\(ext)")
            let budget = min(textExtensions.contains(ext) ? 16 * 1024 * 1024 : 128 * 1024 * 1024, 512 * 1024 * 1024 - inputBytes)
            let data = try readPinned(input, maximumBytes: budget, cancelled: cancelled)
            inputBytes += data.count
            let converted: Data
            do {
                if ext == "pdf" { converted = data }
                else {
                    try data.write(to: staged, options: .withoutOverwriting)
                    let output = directory.appendingPathComponent("converted-\(index).pdf")
                    if imageExtensions.contains(ext) {
                        guard let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary) else { throw ImportFailure.decodeFailed }
                        guard CGImageSourceGetCount(source) > 0, CGImageSourceGetCount(source) <= 500 - pages else { throw ImportFailure.tooLarge }
                        for frame in 0..<CGImageSourceGetCount(source) {
                            guard let props = CGImageSourceCopyPropertiesAtIndex(source, frame, nil) as? [CFString: Any],
                                  let w = (props[kCGImagePropertyPixelWidth] as? NSNumber)?.doubleValue,
                                  let h = (props[kCGImagePropertyPixelHeight] as? NSNumber)?.doubleValue else { throw ImportFailure.decodeFailed }
                            let pixels = try ImagePDFImport.checkedPixelCount(width: w, height: h, maximum: 40_000_000)
                            decodedPixels = try ImagePDFImport.checkedTotal(decodedPixels, adding: pixels, maximum: 100_000_000)
                        }
                        _ = try ImagePDFImport.convert([staged], to: output, cancelled: cancelled)
                    } else { _ = try TextPDFImport.convert(staged, to: output, cancelled: cancelled) }
                    converted = try Data(contentsOf: output)
                }
                guard converted.count <= 256 * 1024 * 1024 - outputBytes else { throw FilePDFImportError.tooLarge }
                guard let doc = PDFDocument(data: converted), doc.pageCount > 0,
                      !doc.isEncrypted, !doc.isLocked, doc.allowsCopying else { throw ImportFailure.invalidInput }
                guard doc.pageCount <= 500 - pages else { throw FilePDFImportError.tooLarge }
                for page in 0..<doc.pageCount {
                    guard doc.page(at: page)?.annotations.contains(where: { $0.type == "Widget" }) != true else { throw ImportFailure.invalidInput }
                }
                pages += doc.pageCount; outputBytes += converted.count
                result.append(converted)
            } catch let error as FilePDFImportError { throw error }
            catch {
                if cancelled() { throw FilePDFImportError.cancelled }
                let reason: String
                switch error as? ImportFailure {
                case .tooLarge: reason = "字节、像素、解压后大小或页数超限"
                case .cancelled: throw FilePDFImportError.cancelled
                case .invalidInput: reason = "含加密/表单 PDF、动态图片或富文档嵌入对象等尚不支持的内容；请先导出 PDF"
                default: reason = "无法解码或格式内容不完整"
                }
                throw FilePDFImportError.failed(name, reason)
            }
        }
        if cancelled() { throw FilePDFImportError.cancelled }
        return result
    }

    private static func readPinned(_ input: URL, maximumBytes: Int, cancelled: () -> Bool) throws -> Data {
        guard input.isFileURL else { throw FilePDFImportError.invalid(input.lastPathComponent) }
        let fd = Darwin.open(input.path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK)
        guard fd >= 0 else { throw FilePDFImportError.invalid(input.lastPathComponent) }
        let file = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        defer { try? file.close() }
        var before = stat()
        guard fstat(fd, &before) == 0, before.st_mode & S_IFMT == S_IFREG else { throw FilePDFImportError.invalid(input.lastPathComponent) }
        guard before.st_size <= maximumBytes else { throw FilePDFImportError.tooLarge }
        let data = try ImagePDFImport.readBounded(file, maximumBytes: maximumBytes, cancelled: cancelled)
        var after = stat()
        guard lstat(input.path, &after) == 0, after.st_mode & S_IFMT == S_IFREG,
              before.st_ino == after.st_ino, before.st_dev == after.st_dev,
              before.st_size == after.st_size, data.count == before.st_size,
              before.st_mtimespec.tv_sec == after.st_mtimespec.tv_sec,
              before.st_mtimespec.tv_nsec == after.st_mtimespec.tv_nsec else { throw FilePDFImportError.invalid(input.lastPathComponent) }
        return data
    }
}
