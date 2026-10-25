import AppKit
import CryptoKit
import Foundation
import PDFKit
import ImageIO
import Darwin

/// Independent PDF object/stream adapter. qpdf is used only for its documented
/// JSON object transport; text and graphics interpretation below is authored here.
final class NativeContentGraph {
    let input: URL
    let document: PDFDocument
    let job: URL
    let qpdf: URL
    let cancelled: () -> Bool
    var objects: [String: Any]
    let pages: [[String: Any]]
    let header: [String: Any]
    var updates: [String: Any] = [:]

    init(input: URL, qpdf: URL, cancelled: @escaping () -> Bool) throws {
        self.input = input; self.qpdf = qpdf; self.cancelled = cancelled
        job = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-NativeContent-\(UUID())")
        guard input.isFileURL, !input.hasDirectoryPath, !input.isFileURL || input.path.hasPrefix("/"),
              let values = try? input.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey]),
              values.isRegularFile == true, values.isSymbolicLink != true,
              let size = values.fileSize, size > 0, size <= 256 * 1024 * 1024,
              let doc = PDFDocument(url: input) else { throw OrganizeError.invalidInput }
        guard !doc.isEncrypted, !doc.isLocked else { throw OrganizeError.encrypted }
        guard doc.pageCount > 0, doc.pageCount <= 10000 else { throw OrganizeError.invalidInput }
        document = doc
        try FileManager.default.createDirectory(at: job, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        do {
            let data = try Self.run(qpdf, ["--json", "--json-key=encrypt", "--json-key=qpdf", "--json-key=pages", "--json-stream-data=inline", "--decode-level=generalized", input.path], in: job, cancelled: cancelled)
            guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let entries = json["qpdf"] as? [[String: Any]], entries.count == 2,
                  let pageEntries = json["pages"] as? [[String: Any]], pageEntries.count == doc.pageCount,
                  let encryption = json["encrypt"] as? [String: Any], encryption["encrypted"] as? Bool == false,
                  entries[1].count <= 100000 else { throw OrganizeError.invalidInput }
            objects = entries[1]; header = entries[0]; pages = pageEntries
            try Self.checkSafety(objects)
            for index in 0..<doc.pageCount {
                guard let page = doc.page(at: index), page.rotation == 0,
                      page.bounds(for: .mediaBox).origin == .zero,
                      page.bounds(for: .mediaBox) == page.bounds(for: .cropBox),
                      page.bounds(for: .mediaBox).width > 0, page.bounds(for: .mediaBox).height > 0,
                      page.bounds(for: .mediaBox).width <= 20000, page.bounds(for: .mediaBox).height <= 20000 else { throw OrganizeError.unsupportedGeometry }
            }
        } catch { try? FileManager.default.removeItem(at: job); throw error }
    }
    deinit { try? FileManager.default.removeItem(at: job) }
    static func checkSafety(_ value: Any) throws {
        if let dictionary = value as? [String: Any] {
            let forbidden: Set<String> = ["/JavaScript", "/JS", "/OpenAction", "/AA", "/EmbeddedFiles", "/Filespec", "/EF", "/Collection", "/OCProperties", "/OCG", "/OCMD", "/OC", "/ActualText", "/Alt", "/StructTreeRoot", "/XFA", "/AcroForm", "/RichMedia", "/Launch", "/SubmitForm", "/ImportData", "/GoToR", "/Sound", "/Movie", "/Encrypt"]
            if dictionary["/Type"] as? String == "/Sig" || dictionary["/FT"] as? String == "/Sig" || dictionary["/ByteRange"] != nil { throw OrganizeError.signed }
            guard Set(dictionary.keys).isDisjoint(with: forbidden) else { throw OrganizeError.processFailed("主动内容、附件、表单或图层文档不支持内容编辑") }
            if let type = dictionary["/Type"] as? String, forbidden.contains(type) { throw OrganizeError.processFailed("不支持的主动内容或图层对象") }
            if let action = dictionary["/S"] as? String, action != "/URI", action != "/GoTo", action != "/Transparency" { throw OrganizeError.processFailed("不支持的 PDF 动作") }
            for child in dictionary.values { try checkSafety(child) }
        } else if let array = value as? [Any] { for child in array { try checkSafety(child) } }
    }
    func dictionary(_ value: Any?) -> [String: Any]? {
        if let direct = value as? [String: Any] { return direct }
        guard let ref = value as? String, let object = objects["obj:" + ref] as? [String: Any] else { return nil }
        return (object["value"] as? [String: Any]) ?? ((object["stream"] as? [String: Any])?["dict"] as? [String: Any])
    }
    func stream(_ reference: String) throws -> Data {
        guard let object = objects["obj:" + reference] as? [String: Any], let stream = object["stream"] as? [String: Any],
              let dictionary = stream["dict"] as? [String: Any], dictionary["/Filter"] == nil,
              let encoded = stream["data"] as? String, encoded.utf8.count <= 96 * 1024 * 1024,
              let bytes = Data(base64Encoded: encoded), bytes.count <= 64 * 1024 * 1024 else {
            throw OrganizeError.processFailed("内容流编码或大小不支持安全编辑")
        }
        return bytes
    }
    func resources(at page: Int) -> [String: Any]? {
        guard var current = dictionary(pages[page]["object"]) else { return nil }
        var visited = Set<String>()
        for _ in 0..<64 {
            if let resources = dictionary(current["/Resources"]) { return resources }
            guard let parent = current["/Parent"] as? String, visited.insert(parent).inserted, let next = dictionary(parent) else { return nil }
            current = next
        }
        return nil
    }
    func pageDictionary(at index: Int) throws -> (String, [String: Any]) {
        guard let reference = pages[index]["object"] as? String, let dictionary = dictionary(reference) else { throw OrganizeError.invalidInput }
        return (reference, dictionary)
    }
    func setObject(_ reference: String, value: [String: Any]) { updates["obj:" + reference] = ["value": value] }
    func setStream(_ reference: String, bytes: Data, dictionary: [String: Any] = [:]) { updates["obj:" + reference] = ["stream": ["dict": dictionary, "data": bytes.base64EncodedString()]] }
    func rewrite(output: URL, overlay: URL? = nil, overlayPage: Int? = nil, validate: (PDFDocument) throws -> Void = { _ in }) throws {
        guard output.isFileURL, !FileManager.default.fileExists(atPath: output.path),
              output.standardizedFileURL != input.standardizedFileURL else { throw OrganizeError.outputExists }
        let update = job.appendingPathComponent("updates.json"), candidate = job.appendingPathComponent("candidate.pdf")
        try JSONSerialization.data(withJSONObject: ["qpdf": [["jsonversion": 2], updates]], options: [.sortedKeys, .withoutEscapingSlashes]).write(to: update, options: [.withoutOverwriting])
        var args = [input.path, "--update-from-json=\(update.path)", "--object-streams=generate", "--compress-streams=y"]
        if let overlay, let overlayPage { args += ["--overlay", overlay.path, "--to=\(overlayPage + 1)", "--from=1", "--"] }
        _ = try Self.run(qpdf, args + [candidate.path], in: job, cancelled: cancelled)
        _ = try Self.run(qpdf, ["--check", candidate.path], in: job, cancelled: cancelled)
        guard let check = PDFDocument(url: candidate), check.pageCount == document.pageCount, !check.isEncrypted else { throw OrganizeError.invalidInput }
        try validate(check)
        if cancelled() { throw OrganizeError.cancelled }
        let staged = output.deletingLastPathComponent().appendingPathComponent(".pdfmaster-content-\(UUID())")
        defer { try? FileManager.default.removeItem(at: staged) }
        try FileManager.default.copyItem(at: candidate, to: staged)
        if cancelled() { throw OrganizeError.cancelled }
        try FileManager.default.linkItem(at: staged, to: output)
    }
    static func run(_ executable: URL, _ arguments: [String], in directory: URL, cancelled: () -> Bool) throws -> Data {
        if cancelled() { throw OrganizeError.cancelled }
        guard FileManager.default.isExecutableFile(atPath: executable.path) else { throw OrganizeError.unavailable }
        let stdout = directory.appendingPathComponent("stdout-\(UUID())"), stderr = directory.appendingPathComponent("stderr-\(UUID())")
        defer { try? FileManager.default.removeItem(at: stdout); try? FileManager.default.removeItem(at: stderr) }
        guard FileManager.default.createFile(atPath: stdout.path, contents: nil, attributes: [.posixPermissions: 0o600]), FileManager.default.createFile(atPath: stderr.path, contents: nil, attributes: [.posixPermissions: 0o600]) else { throw OrganizeError.invalidInput }
        let out = try FileHandle(forWritingTo: stdout), err = try FileHandle(forWritingTo: stderr)
        defer { try? out.close(); try? err.close() }
        let process = Process(); process.executableURL = executable; process.arguments = arguments
        process.standardOutput = out; process.standardError = err; process.standardInput = FileHandle.nullDevice
        process.environment = ["PATH": "/usr/bin:/bin", "LANG": "en_US.UTF-8"]; process.currentDirectoryURL = directory
        try process.run(); let deadline = Date().addingTimeInterval(90)
        while process.isRunning {
            let count = (try? stdout.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
            let errors = (try? stderr.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
            if cancelled() || Date() >= deadline || count > 192 * 1024 * 1024 || errors > 1024 * 1024 {
                process.terminate()
                let stop = Date().addingTimeInterval(1)
                while process.isRunning && Date() < stop { Thread.sleep(forTimeInterval: 0.02) }
                if process.isRunning { kill(process.processIdentifier, SIGKILL) }
                let forcedStop = Date().addingTimeInterval(1)
                while process.isRunning && Date() < forcedStop { Thread.sleep(forTimeInterval:0.01) }
                if cancelled() { throw OrganizeError.cancelled }
                throw OrganizeError.processFailed("原生内容对象处理超时或超出预算")
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
        // waitUntilExit spins a Foundation run loop, which can deliver PDFKit
        // UI notifications on this worker thread. isRunning already observed
        // termination; file-backed output needs only synchronization here.
        try out.synchronize(); try err.synchronize()
        guard process.terminationStatus == 0 else { throw OrganizeError.processFailed(String(decoding: (try Data(contentsOf: stderr)).prefix(1200), as: UTF8.self)) }
        return try Data(contentsOf: stdout)
    }
    static func identity(_ fields: [String]) -> String { SHA256.hash(data: Data(fields.joined(separator: "\0").utf8)).map { String(format: "%02x", $0) }.joined() }
}

struct NativePDFToken {
    enum Value { case number(Double), name(String), bytes(Data), array([NativePDFToken]), word(String) }
    let value: Value; let start: Int; let end: Int
    var number: Double? { if case .number(let value) = value { return value }; return nil }
    var name: String? { if case .name(let value) = value { return value }; return nil }
    var textBytes: Data? {
        switch value { case .bytes(let bytes): return bytes
        case .array(let items): return items.reduce(into: Data()) { if let bytes = $1.textBytes { $0.append(bytes) } }
        default: return nil }
    }
}

/// A bounded lexical reader for PDF content streams. This is not a PDF file
/// parser: qpdf has already resolved objects, filters and stream boundaries.
struct NativePDFLexer {
    let bytes: [UInt8]; var at = 0; var budget = 0
    init(_ data: Data) { bytes = Array(data) }
    mutating func next(depth: Int = 0) throws -> NativePDFToken? {
        guard depth <= 16, budget < 1_000_000 else { throw OrganizeError.invalidInput }
        budget += 1
        while at < bytes.count {
            if [0, 9, 10, 12, 13, 32].contains(bytes[at]) { at += 1; continue }
            if bytes[at] == 37 { while at < bytes.count, ![10, 13].contains(bytes[at]) { at += 1 }; continue }
            break
        }
        guard at < bytes.count else { return nil }
        let start = at, first = bytes[at]; at += 1
        if first == 93 || first == 62 || first == 123 || first == 125 { return NativePDFToken(value:.word(String(UnicodeScalar(first))),start:start,end:at) }
        if first == 60, at < bytes.count, bytes[at] == 60 { at += 1; return NativePDFToken(value:.word("<<"),start:start,end:at) }
        if first == 91 {
            var children: [NativePDFToken] = []
            while let item = try next(depth: depth + 1) {
                if case .word("]") = item.value { return NativePDFToken(value: .array(children), start: start, end: at) }
                children.append(item)
            }
            throw OrganizeError.invalidInput
        }
        if first == 40 {
            var value = Data(), nesting = 1
            while at < bytes.count {
                let b = bytes[at]; at += 1
                if b == 92 {
                    guard at < bytes.count else { throw OrganizeError.invalidInput }
                    let escaped = bytes[at]; at += 1
                    switch escaped {
                    case 110: value.append(10); case 114: value.append(13); case 116: value.append(9); case 98: value.append(8); case 102: value.append(12)
                    case 10: break
                    case 13: if at < bytes.count, bytes[at] == 10 { at += 1 }
                    case 48...55:
                        var code = Int(escaped - 48), count = 1
                        while count < 3, at < bytes.count, (48...55).contains(bytes[at]) { code = code * 8 + Int(bytes[at] - 48); at += 1; count += 1 }
                        value.append(UInt8(code & 255))
                    default: value.append(escaped)
                    }
                } else if b == 40 { nesting += 1; value.append(b) }
                else if b == 41 { nesting -= 1; if nesting == 0 { return NativePDFToken(value: .bytes(value), start: start, end: at) }; value.append(b) }
                else { value.append(b) }
            }
            throw OrganizeError.invalidInput
        }
        if first == 60, at < bytes.count, bytes[at] != 60 {
            var digits = [UInt8]()
            while at < bytes.count, bytes[at] != 62 {
                let b = bytes[at]; at += 1
                if [9, 10, 12, 13, 32].contains(b) { continue }
                guard (48...57).contains(b) || (65...70).contains(b) || (97...102).contains(b) else { throw OrganizeError.invalidInput }
                digits.append(b)
            }
            guard at < bytes.count else { throw OrganizeError.invalidInput }; at += 1
            if digits.count % 2 != 0 { digits.append(48) }
            var value = Data()
            for i in stride(from: 0, to: digits.count, by: 2) { guard let b = UInt8(String(bytes: digits[i..<i+2], encoding: .ascii)!, radix: 16) else { throw OrganizeError.invalidInput }; value.append(b) }
            return NativePDFToken(value: .bytes(value), start: start, end: at)
        }
        let delimiters: Set<UInt8> = [0,9,10,12,13,32,40,41,60,62,91,93,123,125,47,37]
        if first == 47 {
            let nameStart = at
            while at < bytes.count, !delimiters.contains(bytes[at]) { at += 1 }
            var decoded:[UInt8] = [], index = nameStart
            while index < at {
                if bytes[index] == 35, index+2 < at, let value = UInt8(String(decoding:bytes[index+1...index+2],as:UTF8.self),radix:16) { decoded.append(value); index += 3 }
                else { decoded.append(bytes[index]); index += 1 }
            }
            return NativePDFToken(value: .name("/" + String(decoding: decoded, as: UTF8.self)), start: start, end: at)
        }
        while at < bytes.count, !delimiters.contains(bytes[at]) { at += 1 }
        let value = String(decoding: bytes[start..<at], as: UTF8.self)
        if let number = Double(value) {
            guard number.isFinite, abs(number) <= 1e9 else { throw OrganizeError.invalidInput }
            return NativePDFToken(value:.number(number),start:start,end:at)
        }
        return NativePDFToken(value:.word(value),start:start,end:at)
    }
}
