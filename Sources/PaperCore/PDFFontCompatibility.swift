import Foundation
import PDFKit
import Darwin

private final class FontDictionaryValues { var values: [CGPDFObjectRef] = [] }
private func collectFontValues(_ key: UnsafePointer<CChar>, _ value: CGPDFObjectRef, _ context: UnsafeMutableRawPointer?) {
    guard let context else { return }
    Unmanaged<FontDictionaryValues>.fromOpaque(context).takeUnretainedValue().values.append(value)
}

/// Repairs only positively identified malformed embedded CFF, in a private copy.
/// No network, source writes, system font changes, or rasterized page fallback.
enum PDFFontCompatibility {
    struct Result { let data: Data; let notice: String? }

    static func normalize(_ data: Data) -> Result {
        guard needsRepair(data) else { return Result(data: data, notice: nil) }
        let unresolved = "嵌入字体兼容修复未完成，部分文字可能显示异常；原文件未改动"
        do {
            let bundled = Bundle.main.resourceURL?.appendingPathComponent("Engines/content-edit/font_compatibility.py")
            let local = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("Resources/ContentEdit/font_compatibility.py")
            let script = bundled.flatMap { FileManager.default.fileExists(atPath: $0.path) ? $0 : nil } ?? local
            guard FileManager.default.isExecutableFile(atPath: ContentEditEngine.localPython.path),
                  FileManager.default.fileExists(atPath: script.path) else { return Result(data: data, notice: unresolved) }
            let dir = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-Font-" + UUID().uuidString)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
            defer { try? FileManager.default.removeItem(at: dir) }
            let input = dir.appendingPathComponent("input.pdf"), output = dir.appendingPathComponent("output.pdf")
            let logURL = dir.appendingPathComponent("engine.log")
            guard FileManager.default.createFile(atPath: input.path, contents: data, attributes: [.posixPermissions: 0o600]),
                  FileManager.default.createFile(atPath: logURL.path, contents: nil, attributes: [.posixPermissions: 0o600]) else {
                return Result(data: data, notice: unresolved)
            }
            let log = try FileHandle(forWritingTo: logURL); defer { try? log.close() }
            let process = Process(); process.executableURL = ContentEditEngine.localPython
            process.arguments = ["-B", "-I", "-S", script.path, "--input", input.path, "--output", output.path]
            process.environment = ["PATH": "/usr/bin:/bin", "LANG": "en_US.UTF-8", "PYTHONDONTWRITEBYTECODE": "1"]
            process.currentDirectoryURL = dir; process.standardInput = FileHandle.nullDevice
            process.standardOutput = log; process.standardError = log
            try process.run()
            let deadline = Date().addingTimeInterval(25)
            while process.isRunning {
                let size = (try? logURL.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
                if Date() >= deadline || size > 1024 * 1024 {
                    process.terminate()
                    let stop = Date().addingTimeInterval(0.5)
                    while process.isRunning, Date() < stop { Thread.sleep(forTimeInterval: 0.01) }
                    if process.isRunning { kill(process.processIdentifier, SIGKILL) }
                    process.waitUntilExit()
                    return Result(data: data, notice: unresolved)
                }
                Thread.sleep(forTimeInterval: 0.02)
            }
            process.waitUntilExit()
            guard process.terminationStatus == 0,
                  let size = try? output.resourceValues(forKeys: [.fileSizeKey]).fileSize,
                  size > 0, size <= 256 * 1024 * 1024 else { return Result(data: data, notice: unresolved) }
            let fixed = try Data(contentsOf: output)
            guard let repaired = PDFDocument(data: fixed), !repaired.isEncrypted,
                  repaired.pageCount == PDFDocument(data: data)?.pageCount, !needsRepair(fixed) else {
                return Result(data: data, notice: unresolved)
            }
            return Result(data: fixed, notice: "已兼容修复异常嵌入字体 · 原文件未改动")
        } catch { return Result(data: data, notice: unresolved) }
    }

    static func needsRepair(_ data: Data) -> Bool {
        guard let provider = CGDataProvider(data: data as CFData), let pdf = CGPDFDocument(provider), !pdf.isEncrypted else { return false }
        var visited: Set<UInt> = []
        func resources(_ dictionary: CGPDFDictionaryRef, depth: Int) -> Bool {
            guard depth < 32, visited.insert(UInt(bitPattern: dictionary.rawValue)).inserted else { return false }
            var fonts: CGPDFDictionaryRef?
            if CGPDFDictionaryGetDictionary(dictionary, "Font", &fonts), let fonts {
                let values = FontDictionaryValues()
                CGPDFDictionaryApplyFunction(fonts, collectFontValues, Unmanaged.passUnretained(values).toOpaque())
                for object in values.values {
                    var font: CGPDFDictionaryRef?
                    guard CGPDFObjectGetValue(object, .dictionary, &font), let font else { continue }
                    var descendants: CGPDFArrayRef?
                    guard CGPDFDictionaryGetArray(font, "DescendantFonts", &descendants), let descendants else { continue }
                    for i in 0..<CGPDFArrayGetCount(descendants) {
                        var child: CGPDFDictionaryRef?, descriptor: CGPDFDictionaryRef?, stream: CGPDFStreamRef?
                        var subtype: UnsafePointer<CChar>?
                        var format = CGPDFDataFormat.raw
                        guard CGPDFArrayGetDictionary(descendants, i, &child), let child,
                              CGPDFDictionaryGetName(child, "Subtype", &subtype), let subtype, String(cString: subtype) == "CIDFontType2",
                              CGPDFDictionaryGetDictionary(child, "FontDescriptor", &descriptor), let descriptor,
                              CGPDFDictionaryGetStream(descriptor, "FontFile2", &stream), let stream,
                              let bytes = CGPDFStreamCopyData(stream, &format) as Data?, bytes.count >= 4 else { continue }
                        if bytes[0] == 1, bytes[1] == 0 { return true }
                    }
                }
            }
            var forms: CGPDFDictionaryRef?
            if CGPDFDictionaryGetDictionary(dictionary, "XObject", &forms), let forms {
                let values = FontDictionaryValues()
                CGPDFDictionaryApplyFunction(forms, collectFontValues, Unmanaged.passUnretained(values).toOpaque())
                for object in values.values {
                    var stream: CGPDFStreamRef?, nested: CGPDFDictionaryRef?
                    if CGPDFObjectGetValue(object, .stream, &stream), let stream, let streamDictionary = CGPDFStreamGetDictionary(stream),
                       CGPDFDictionaryGetDictionary(streamDictionary, "Resources", &nested), let nested,
                       resources(nested, depth: depth + 1) { return true }
                }
            }
            return false
        }
        for i in 1...max(1,pdf.numberOfPages) {
            var page = pdf.page(at: i)?.dictionary
            for _ in 0..<32 {
                guard let current = page else { break }
                var dictionary: CGPDFDictionaryRef?
                if CGPDFDictionaryGetDictionary(current, "Resources", &dictionary), let dictionary,
                   resources(dictionary, depth: 0) { return true }
                var parent: CGPDFDictionaryRef?
                _ = CGPDFDictionaryGetDictionary(current, "Parent", &parent); page = parent
            }
        }
        return false
    }
}
