import Foundation
import AppKit
import CoreText
import CoreGraphics
import Darwin

/// Offline text-only import: UTF-8/BOM UTF-16 TXT and literal Markdown, native
/// RTF and DOCX. Rich documents are reflowed, not reproduced with Office parity.
/// Attachments are rejected rather than silently omitted. Call off the UI thread.
/// Cancellation cannot interrupt an individual system document decode/layout.
/// Unified CJK uses the native Arial Unicode font for stable Unicode after PDFKit
/// page copying/saving; CJK import is rejected if that native font is unavailable.
public enum TextPDFImport {
    private static let maximumInputBytes = 16 * 1024 * 1024
    private static let maximumPages = 1_000
    private static let maximumOutputBytes = 256 * 1024 * 1024

    public static func convert(_ input: URL, to output: URL, cancelled: () -> Bool = { false }) throws -> Int {
        guard input.isFileURL, output.isFileURL, output.pathExtension.lowercased() == "pdf",
              ["txt", "md", "rtf", "docx"].contains(input.pathExtension.lowercased()) else { throw ImportFailure.invalidInput }
        var existing = stat()
        guard lstat(output.path, &existing) != 0, errno == ENOENT else { throw ImportFailure.outputExists }
        if cancelled() { throw ImportFailure.cancelled }
        let data = try readInput(input, cancelled: cancelled)
        let text = try decode(data, extension: input.pathExtension.lowercased())
        if cancelled() { throw ImportFailure.cancelled }
        let framesetter = CTFramesetterCreateWithAttributedString(text)
        let pageBox = CGRect(x: 0, y: 0, width: 595.276, height: 841.89)
        let contentBox = pageBox.insetBy(dx: 24, dy: 24)
        let path = CGPath(rect: contentBox, transform: nil)
        var ranges: [CFRange] = []
        var offset = 0
        repeat {
            if cancelled() { throw ImportFailure.cancelled }
            guard ranges.count < maximumPages else { throw ImportFailure.tooLarge }
            let frame = CTFramesetterCreateFrame(framesetter, CFRange(location: offset, length: 0), path, nil)
            let visible = CTFrameGetVisibleStringRange(frame)
            guard text.length == 0 || visible.length > 0 else { throw ImportFailure.tooLarge }
            ranges.append(visible)
            offset += visible.length
        } while offset < text.length

        let parent = output.deletingLastPathComponent().resolvingSymlinksInPath()
        var directory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: parent.path, isDirectory: &directory), directory.boolValue else { throw ImportFailure.invalidInput }
        let target = parent.appendingPathComponent(output.lastPathComponent)
        let stage = parent.appendingPathComponent(".text-import-\(UUID().uuidString).pdf")
        let fd = Darwin.open(stage.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
        guard fd >= 0 else { throw ImportFailure.writeFailed }
        defer { close(fd); try? FileManager.default.removeItem(at: stage) }
        let writer = BoundedPDFWriter(fd: fd, maximumBytes: maximumOutputBytes)
        var callbacks = CGDataConsumerCallbacks(putBytes: { info, bytes, count in
            guard let info else { return 0 }
            return Unmanaged<BoundedPDFWriter>.fromOpaque(info).takeUnretainedValue().write(bytes, count: count)
        }, releaseConsumer: nil)
        guard let consumer = CGDataConsumer(info: Unmanaged.passUnretained(writer).toOpaque(), cbks: &callbacks),
              let context = CGContext(consumer: consumer, mediaBox: nil,
                                      [kCGPDFContextCreator: "PDF Master local text import"] as CFDictionary) else { throw ImportFailure.writeFailed }
        var closed = false
        defer { if !closed { context.closePDF() }; withExtendedLifetime(writer) {} }
        for range in ranges {
            if cancelled() { throw ImportFailure.cancelled }
            var box = pageBox
            let boxData = Data(bytes: &box, count: MemoryLayout<CGRect>.size)
            context.beginPDFPage([kCGPDFContextMediaBox: boxData] as CFDictionary)
            context.setFillColor(CGColor(gray: 1, alpha: 1))
            context.fill(pageBox)
            context.textMatrix = .identity
            let frame = CTFramesetterCreateFrame(framesetter, range, path, nil)
            CTFrameDraw(frame, context)
            context.endPDFPage()
            if let failure = writer.failure { throw failure }
        }
        context.closePDF(); closed = true
        if let failure = writer.failure { throw failure }
        guard fsync(fd) == 0, let check = CGPDFDocument(stage as CFURL), check.numberOfPages == ranges.count else { throw ImportFailure.writeFailed }
        if cancelled() { throw ImportFailure.cancelled }
        // Atomic create-if-absent also protects a target created after preflight.
        guard link(stage.path, target.path) == 0 else { throw errno == EEXIST ? ImportFailure.outputExists : ImportFailure.writeFailed }
        return ranges.count
    }

    private static func readInput(_ input: URL, cancelled: () -> Bool) throws -> Data {
        let fd = Darwin.open(input.path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK)
        guard fd >= 0 else { throw ImportFailure.invalidInput }
        let file = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        defer { try? file.close() }
        var info = stat()
        guard fstat(fd, &info) == 0, (info.st_mode & S_IFMT) == S_IFREG, info.st_size >= 0 else { throw ImportFailure.invalidInput }
        guard info.st_size <= maximumInputBytes else { throw ImportFailure.tooLarge }
        return try ImagePDFImport.readBounded(file, maximumBytes: maximumInputBytes, cancelled: cancelled)
    }

    private static func decode(_ data: Data, extension fileExtension: String) throws -> NSAttributedString {
        let imported: NSAttributedString
        if fileExtension == "txt" || fileExtension == "md" {
            let string: String?
            if data.starts(with: [0xff, 0xfe]) || data.starts(with: [0xfe, 0xff]) {
                string = data.count % 2 == 0 ? String(data: data, encoding: .utf16) : nil
            } else {
                string = String(data: data, encoding: .utf8)
            }
            guard let string else { throw ImportFailure.decodeFailed }
            imported = NSAttributedString(string: string.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n"))
        } else {
            let type: NSAttributedString.DocumentType = fileExtension == "rtf" ? .rtf : .officeOpenXML
            if fileExtension == "rtf" {
                guard data.starts(with: Data("{\\rtf".utf8)) else { throw ImportFailure.decodeFailed }
                try rejectRTFObjects(data)
            } else {
                try preflightDOCX(data)
            }
            do {
                imported = try NSAttributedString(data: data, options: [.documentType: type], documentAttributes: nil)
            } catch { throw ImportFailure.decodeFailed }
        }
        guard imported.length <= maximumInputBytes else { throw ImportFailure.tooLarge }
        let rendered = NSMutableAttributedString(string: imported.string, attributes: [
            .font: NSFont.systemFont(ofSize: 12),
            .foregroundColor: CGColor(gray: 0, alpha: 1)
        ])
        var hasAttachment = false
        imported.enumerateAttributes(in: NSRange(location: 0, length: imported.length)) { attributes, range, _ in
            if attributes[.attachment] != nil { hasAttachment = true }
            if let font = attributes[.font] as? NSFont {
                let size = font.pointSize
                let safeSize = size.isFinite ? min(72, max(6, size)) : 12
                rendered.addAttribute(.font, value: NSFont(name: font.fontName, size: safeSize) ?? NSFont.systemFont(ofSize: safeSize), range: range)
            }
            for key in [NSAttributedString.Key.paragraphStyle, .underlineStyle, .strikethroughStyle] {
                if let value = attributes[key] { rendered.addAttribute(key, value: value, range: range) }
            }
        }
        guard !hasAttachment else { throw ImportFailure.invalidInput }
        // PingFang/STSong share glyph IDs between Unified CJK and Kangxi radicals.
        // PDFKit rebuilds their ToUnicode table on save and selects radical codepoints
        // (入 U+5165 -> ⼊ U+2F0A), corrupting literal searches despite identical pixels.
        // Arial Unicode has independent mappings for the Unified CJK characters.
        let units = Array(imported.string.utf16)
        var index = 0
        while index < units.count {
            guard (0x3400...0x9fff).contains(units[index]) || (0xf900...0xfaff).contains(units[index]) else { index += 1; continue }
            let start = index
            while index < units.count, (0x3400...0x9fff).contains(units[index]) || (0xf900...0xfaff).contains(units[index]) { index += 1 }
            let range = NSRange(location: start, length: index - start)
            var missingStableFont = false
            rendered.enumerateAttribute(.font, in: range) { value, fontRange, _ in
                let size = (value as? NSFont)?.pointSize ?? 12
                guard let font = NSFont(name: "ArialUnicodeMS", size: size) else { missingStableFont = true; return }
                rendered.addAttribute(.font, value: font, range: fontRange)
            }
            if missingStableFont { throw ImportFailure.decodeFailed }
        }
        return rendered
    }

    private static func rejectRTFObjects(_ data: Data) throws {
        var index = 0
        while index < data.count {
            guard data[index] == 92 else { index += 1; continue }
            index += 1
            let start = index
            while index < data.count, (65...90).contains(data[index]) || (97...122).contains(data[index]) { index += 1 }
            if index == start { index += 1; continue } // Escaped backslash/braces are literal text.
            let word = String(decoding: data[start..<index], as: UTF8.self).lowercased()
            if ["pict", "object", "objdata", "nextgraphic"].contains(word) { throw ImportFailure.invalidInput }
        }
    }

    /// Native Office import can expand a small ZIP considerably. Reject oversized
    /// declared members before handing it to AppKit; this bounds archive metadata,
    /// not the system importer's internal allocation or execution time.
    private static func preflightDOCX(_ data: Data) throws {
        guard data.count >= 22, data.starts(with: [0x50, 0x4b]) else { throw ImportFailure.decodeFailed }
        func number(_ offset: Int, _ bytes: Int) -> Int {
            guard offset >= 0, offset <= data.count - bytes else { return -1 }
            var value = 0
            for index in 0..<bytes { value |= Int(data[offset + index]) << (8 * index) }
            return value
        }
        let minimum = max(0, data.count - 65_557)
        guard let end = stride(from: data.count - 22, through: minimum, by: -1).first(where: {
            number($0, 4) == 0x06054b50 && $0 + 22 + number($0 + 20, 2) == data.count
        }) else { throw ImportFailure.decodeFailed }
        guard number(end + 4, 2) == 0, number(end + 6, 2) == 0,
              number(end + 8, 2) == number(end + 10, 2) else { throw ImportFailure.invalidInput }
        let entries = number(end + 10, 2)
        guard entries > 0, entries <= 1_024 else { throw ImportFailure.tooLarge }
        let directorySize = number(end + 12, 4)
        var cursor = number(end + 16, 4)
        guard cursor >= 0, directorySize >= 0, cursor <= end, directorySize <= end - cursor else { throw ImportFailure.decodeFailed }
        let directoryEnd = cursor + directorySize
        var expandedBytes = 0
        var names = Set<String>()
        for _ in 0..<entries {
            guard cursor <= directoryEnd - 46, number(cursor, 4) == 0x02014b50 else { throw ImportFailure.decodeFailed }
            let flags = number(cursor + 8, 2)
            let method = number(cursor + 10, 2)
            let compressed = number(cursor + 20, 4)
            let expanded = number(cursor + 24, 4)
            let nameLength = number(cursor + 28, 2)
            let extraLength = number(cursor + 30, 2)
            let commentLength = number(cursor + 32, 2)
            let recordLength = 46 + nameLength + extraLength + commentLength
            guard recordLength >= 46, recordLength <= directoryEnd - cursor else { throw ImportFailure.decodeFailed }
            guard flags & 1 == 0, method == 0 || method == 8,
                  number(cursor + 34, 2) == 0 else { throw ImportFailure.invalidInput }
            guard compressed <= maximumInputBytes, expanded <= maximumInputBytes,
                  expanded <= 64 * 1024 * 1024 - expandedBytes else { throw ImportFailure.tooLarge }
            expandedBytes += expanded
            guard let name = String(data: data[(cursor + 46)..<(cursor + 46 + nameLength)], encoding: .utf8),
                  !name.hasPrefix("/"), !name.split(separator: "/").contains(".."), names.insert(name).inserted else { throw ImportFailure.invalidInput }
            let lowerName = name.lowercased()
            if lowerName.hasPrefix("word/media/") || lowerName.hasPrefix("word/embeddings/") { throw ImportFailure.invalidInput }
            let local = number(cursor + 42, 4)
            guard local >= 0, local <= data.count - 30, number(local, 4) == 0x04034b50 else { throw ImportFailure.decodeFailed }
            guard number(local + 18, 4) <= maximumInputBytes, number(local + 22, 4) <= maximumInputBytes else { throw ImportFailure.tooLarge }
            let localNameLength = number(local + 26, 2)
            let localExtraLength = number(local + 28, 2)
            let payload = local + 30 + localNameLength + localExtraLength
            guard payload <= data.count, compressed <= data.count - payload, localNameLength == nameLength,
                  data[(local + 30)..<(local + 30 + localNameLength)] == data[(cursor + 46)..<(cursor + 46 + nameLength)] else { throw ImportFailure.decodeFailed }
            cursor += recordLength
        }
        guard cursor == directoryEnd, names.contains("word/document.xml"), names.contains("[Content_Types].xml") else { throw ImportFailure.decodeFailed }
    }

    /// A hard byte ceiling on bytes actually written, even when CGPDF buffers data.
    private final class BoundedPDFWriter {
        let fd: Int32
        let maximumBytes: Int
        var written = 0
        var failure: ImportFailure?
        init(fd: Int32, maximumBytes: Int) { self.fd = fd; self.maximumBytes = maximumBytes }
        func write(_ bytes: UnsafeRawPointer, count: Int) -> Int {
            guard failure == nil else { return 0 }
            guard count <= maximumBytes - written else { failure = .tooLarge; return 0 }
            var offset = 0
            while offset < count {
                let result = Darwin.write(fd, bytes.advanced(by: offset), count - offset)
                if result < 0 && errno == EINTR { continue }
                guard result > 0 else { failure = .writeFailed; return offset }
                offset += result
            }
            written += count
            return count
        }
    }
}
