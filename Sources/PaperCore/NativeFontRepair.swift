import Foundation
import PDFKit
import Darwin

/// Corrects a narrowly identified PDF font-container error without changing the
/// original CFF outlines. qpdf is the already bundled Apache-2.0 object writer.
enum NativeFontRepair {
    enum Invalid: Error { case unavailable, input, unsupported, process(String) }

    static func repair(_ source: Data) throws -> Data {
        guard !source.isEmpty, source.count <= 256 * 1024 * 1024 else { throw Invalid.input }
        guard let executable = executable() else { throw Invalid.unavailable }
        let job = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-NativeFont-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: job, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: job) }
        let input = job.appendingPathComponent("input.pdf"), output = job.appendingPathComponent("fixed.pdf")
        try source.write(to: input, options: .withoutOverwriting)
        let runner = Runner(executable: executable, job: job, jobDeadline: Date().addingTimeInterval(35))
        let data = try runner.run([input.path, "--json", "--json-stream-data=none"], limit: 32 * 1024 * 1024)
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let encryption = json["encrypt"] as? [String: Any], encryption["encrypted"] as? Bool == false,
              let sections = json["qpdf"] as? [Any], sections.count == 2,
              let header = sections[0] as? [String: Any], let objects = sections[1] as? [String: Any],
              let maximum = header["maxobjectid"] as? Int, maximum > 0, maximum <= 100_000,
              !hasSignature(objects) else { throw Invalid.unsupported }
        var replacements: [String: Any] = [:], next = maximum, inspectedFonts = 0
        func allocate(_ value: [String: Any]) -> String {
            next += 1; let reference = "\(next) 0 R"
            replacements["obj:" + reference] = value
            return reference
        }
        func dictionary(_ reference: String) -> [String: Any]? {
            let object = objects["obj:" + reference] as? [String: Any]
            if let value = object?["value"] as? [String: Any] { return value }
            return (object?["stream"] as? [String: Any])?["dict"] as? [String: Any]
        }
        func stream(_ reference: String) throws -> Data {
            let parts = reference.split(separator: " ")
            guard parts.count == 3, parts[2] == "R", Int(parts[0]) != nil, Int(parts[1]) != nil else { throw Invalid.input }
            return try runner.run([input.path, "--show-object=\(parts[0]),\(parts[1])", "--filtered-stream-data"], limit: 16 * 1024 * 1024)
        }
        for key in objects.keys.sorted() {
            guard let object = objects[key] as? [String: Any], var parent = object["value"] as? [String: Any],
                  parent["/Subtype"] as? String == "/Type0",
                  let children = parent["/DescendantFonts"] as? [String], children.count == 1,
                  var child = dictionary(children[0]), child["/Subtype"] as? String == "/CIDFontType2",
                  let descriptorRef = child["/FontDescriptor"] as? String,
                  var descriptor = dictionary(descriptorRef), let programRef = descriptor["/FontFile2"] as? String else { continue }
            inspectedFonts += 1
            guard inspectedFonts <= 64 else { throw Invalid.unsupported }
            let program = try stream(programRef)
            guard program.count >= 4, program[0] == 1, program[1] == 0 else { continue }
            guard parent["/Encoding"] as? String == "/Identity-H",
                  child["/W2"] == nil, child["/DW2"] == nil,
                  let mapRef = child["/CIDToGIDMap"] as? String,
                  let system = child["/CIDSystemInfo"] as? [String: Any] else { throw Invalid.unsupported }
            let charset = try NativeFontCFF.charset(program), mapping = try stream(mapRef)
            guard system["/Registry"] as? String == "u:" + charset.registry,
                  system["/Ordering"] as? String == "u:" + charset.ordering,
                  system["/Supplement"] as? Int == charset.supplement,
                  !mapping.isEmpty, mapping.count.isMultiple(of: 2), mapping.count <= 131_072 else { throw Invalid.unsupported }
            var cids: [Int] = []
            for at in stride(from: 0, to: mapping.count, by: 2) {
                let cid = Int(mapping[at]) * 256 + Int(mapping[at + 1]), glyph = at / 2
                guard cid == 0 || (glyph < charset.cids.count && charset.cids[glyph] == cid) else { throw Invalid.unsupported }
                cids.append(cid)
            }
            // This defective wrapper uses GID-indexed codes. Change only the
            // CFF charset labels; preserve outlines, metrics and ToUnicode.
            guard cids == charset.cids else { throw Invalid.unsupported }
            descriptor.removeValue(forKey: "/FontFile2")
            guard descriptor["/FontFile"] == nil, descriptor["/FontFile3"] == nil else { throw Invalid.unsupported }
            descriptor["/FontFile3"] = allocate(["stream": ["dict": ["/Subtype": "/CIDFontType0C"], "data": charset.denseProgram.base64EncodedString()]])
            child["/FontDescriptor"] = allocate(["value": descriptor])
            child["/Subtype"] = "/CIDFontType0"
            child.removeValue(forKey: "/CIDToGIDMap")
            parent["/DescendantFonts"] = [allocate(["value": child])]
            replacements[key] = ["value": parent]
        }
        guard !replacements.isEmpty, next <= 200_000 else { throw Invalid.unsupported }
        let update = job.appendingPathComponent("font-update.json")
        let patch = try JSONSerialization.data(withJSONObject: ["qpdf": [header, replacements]], options: [.sortedKeys, .withoutEscapingSlashes])
        guard patch.count <= 32 * 1024 * 1024 else { throw Invalid.input }
        try patch.write(to: update, options: .withoutOverwriting)
        _ = try runner.run([input.path, "--update-from-json=" + update.path, output.path], limit: 1024 * 1024)
        let size = try output.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        guard size > 0, size <= 256 * 1024 * 1024 else { throw Invalid.input }
        return try Data(contentsOf: output)
    }

    private static func hasSignature(_ value: Any) -> Bool {
        if let dictionary = value as? [String: Any] {
            if dictionary["/Type"] as? String == "/Sig" || dictionary["/FT"] as? String == "/Sig" || dictionary["/ByteRange"] != nil { return true }
            return dictionary.values.contains(where: hasSignature)
        }
        return (value as? [Any])?.contains(where: hasSignature) ?? false
    }

    private static func executable() -> URL? {
        var candidates: [URL] = []
        if let resources = Bundle.main.resourceURL { candidates.append(resources.appendingPathComponent("Engines/qpdf/bin/qpdf")) }
        #if DEBUG
        if let path = ProcessInfo.processInfo.environment["PDFMASTER_TEST_QPDF"], path.hasPrefix("/") { candidates.append(URL(fileURLWithPath: path)) }
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        candidates.append(root.appendingPathComponent("tools/qpdf-arm64-native-jpeg8-12.4.2/bin/qpdf"))
        #endif
        return candidates.first { FileManager.default.isExecutableFile(atPath: $0.path) }
    }

    private struct Runner {
        let executable: URL
        let job: URL
        let jobDeadline: Date
        func run(_ arguments: [String], limit: Int) throws -> Data {
            guard Date() < jobDeadline else { throw Invalid.process("Job budget exceeded") }
            let stdout = job.appendingPathComponent("out-" + UUID().uuidString)
            let stderr = job.appendingPathComponent("err-" + UUID().uuidString)
            guard FileManager.default.createFile(atPath: stdout.path, contents: nil, attributes: [.posixPermissions: 0o600]),
                  FileManager.default.createFile(atPath: stderr.path, contents: nil, attributes: [.posixPermissions: 0o600]) else { throw Invalid.input }
            defer { try? FileManager.default.removeItem(at: stdout); try? FileManager.default.removeItem(at: stderr) }
            let out = try FileHandle(forWritingTo: stdout), error = try FileHandle(forWritingTo: stderr)
            defer { try? out.close(); try? error.close() }
            let task = Process(); task.executableURL = executable; task.arguments = arguments
            task.environment = ["PATH": "/usr/bin:/bin", "LANG": "en_US.UTF-8"]
            task.currentDirectoryURL = job; task.standardInput = FileHandle.nullDevice
            task.standardOutput = out; task.standardError = error
            try task.run()
            let deadline = min(Date().addingTimeInterval(25), jobDeadline)
            while task.isRunning {
                let count = (try? stdout.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
                let errorCount = (try? stderr.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
                if Date() > deadline || count > limit || errorCount > 1024 * 1024 {
                    task.terminate()
                    let grace = Date().addingTimeInterval(0.5)
                    while task.isRunning, Date() < grace { Thread.sleep(forTimeInterval: 0.01) }
                    if task.isRunning { kill(task.processIdentifier, SIGKILL) }
                    let forcedStop = Date().addingTimeInterval(1)
                    while task.isRunning, Date() < forcedStop { Thread.sleep(forTimeInterval:0.01) }
                    throw Invalid.process("Process budget exceeded")
                }
                Thread.sleep(forTimeInterval: 0.01)
            }
            // The isRunning loop already observed termination. Do not spin a
            // Foundation wait run loop here: PDFKit can deliver coalesced view
            // notifications on this worker and violate AppKit thread affinity.
            try out.synchronize(); try error.synchronize()
            guard task.terminationStatus == 0,
                  (try stdout.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0) <= limit else {
                let message = String(data: (try? Data(contentsOf: stderr)) ?? Data(), encoding: .utf8) ?? "Object writer refused input"
                throw Invalid.process(String(message.prefix(600)))
            }
            return try Data(contentsOf: stdout)
        }
    }
}
