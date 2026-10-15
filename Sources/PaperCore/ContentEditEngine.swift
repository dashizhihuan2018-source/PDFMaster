import Foundation
import Darwin

public struct ContentSpan: Codable { public let id: String; public let text: String; public let bbox: [Double]; public let origin: [Double]; public let font: String; public let size: Double; public let color: Int }
public struct ContentImage: Codable { public let id: String; public let xref: Int; public let bbox: [Double] }
public struct ContentLink: Codable { public let id: String; public let bbox: [Double]; public let uri: String }
public struct ContentPage: Codable {
    public let page: Int; public let width: Double; public let height: Double
    public let spans: [ContentSpan]; public let images: [ContentImage]; public let links: [ContentLink]; public let limitations: [String]
}
public struct ContentEditRequest: Encodable {
    public let operation: String; public let page: Int
    public var id: String?; public var text: String?; public var bbox: [Double]?; public var uri: String?; public var image_path: String?
    public init(operation: String, page: Int) { self.operation = operation; self.page = page }
}

/// Local research adapter. PyMuPDF is an explicitly external runtime, not bundled or
/// licensed for closed-source distribution by this app. No network or shell commands.
public final class ContentEditEngine {
    public static let localPython = URL(fileURLWithPath: "/Library/Frameworks/Python.framework/Versions/3.12/bin/python3")
    public let python: URL; public let script: URL
    public init(python: URL = ContentEditEngine.localPython, script: URL) { self.python = python; self.script = script }

    public func validateAdmission(_ data: Data, in directory: URL, cancelled: () -> Bool) throws {
        let source = directory.appendingPathComponent("admission-" + UUID().uuidString + ".pdf")
        guard FileManager.default.createFile(atPath: source.path, contents: data, attributes: [.posixPermissions: 0o600]) else { throw OrganizeError.invalidInput }
        defer { try? FileManager.default.removeItem(at: source) }
        _ = try inspect(source, page: 0, cancelled: cancelled)
    }

    public func inspect(_ source: URL, page: Int, cancelled: () -> Bool) throws -> ContentPage {
        let data = try run(source, request: ContentEditRequest(operation: "inspect", page: page), output: nil, cancelled: cancelled)
        let result = try JSONDecoder().decode(ContentPage.self, from: data)
        guard result.page == page, result.width.isFinite, result.height.isFinite,
              result.width > 0, result.height > 0, result.width <= 20000, result.height <= 20000,
              result.spans.count <= 20000, result.images.count <= 10000, result.links.count <= 10000,
              result.spans.allSatisfy({ validBox($0.bbox, in: result) && $0.size.isFinite && $0.size > 0 && $0.text.utf8.count <= 100000 }),
              result.images.allSatisfy({ validBox($0.bbox, in: result) }), result.links.allSatisfy({ validBox($0.bbox, in: result) }) else { throw OrganizeError.invalidInput }
        return result
    }
    public func apply(_ source: URL, request: ContentEditRequest, output: URL, cancelled: () -> Bool) throws -> String {
        guard !FileManager.default.fileExists(atPath: output.path), request.operation != "inspect" else { throw OrganizeError.invalidInput }
        let data = try run(source, request: request, output: output, cancelled: cancelled)
        let json = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        return (json?["message"] as? String) ?? "内容编辑已完成 · 可撤销 · 请存储副本"
    }
    private func validBox(_ box: [Double], in page: ContentPage) -> Bool {
        box.count == 4 && box.allSatisfy(\.isFinite) && box[0] >= -1 && box[1] >= -1 && box[2] > box[0] && box[3] > box[1] && box[2] <= page.width + 1 && box[3] <= page.height + 1
    }
    private func run(_ source: URL, request: ContentEditRequest, output: URL?, cancelled: () -> Bool) throws -> Data {
        guard FileManager.default.isExecutableFile(atPath: python.path), FileManager.default.fileExists(atPath: script.path), !cancelled() else { throw OrganizeError.unavailable }
        let dir = source.deletingLastPathComponent()
        let requestURL = dir.appendingPathComponent("request-" + UUID().uuidString + ".json")
        let responseURL = dir.appendingPathComponent("response-" + UUID().uuidString + ".json")
        let logURL = dir.appendingPathComponent("engine-" + UUID().uuidString + ".log")
        defer { for file in [requestURL, responseURL, logURL] { try? FileManager.default.removeItem(at: file) } }
        let encoded = try JSONEncoder().encode(request)
        guard encoded.count <= 1024 * 1024 else { throw OrganizeError.invalidInput }
        guard FileManager.default.createFile(atPath: requestURL.path, contents: encoded, attributes: [.posixPermissions: 0o600]),
              FileManager.default.createFile(atPath: logURL.path, contents: nil, attributes: [.posixPermissions: 0o600]) else { throw OrganizeError.invalidInput }
        let log = try FileHandle(forWritingTo: logURL); defer { try? log.close() }
        let process = Process(); process.executableURL = python
        process.arguments = ["-I", "-S", script.path, "--input", source.path, "--request", requestURL.path, "--response", responseURL.path] + (output.map { ["--output", $0.path] } ?? [])
        process.environment = ["PATH": "/usr/bin:/bin", "LANG": "en_US.UTF-8"]
        process.currentDirectoryURL = dir; process.standardInput = FileHandle.nullDevice
        process.standardOutput = log; process.standardError = log
        try process.run()
        let deadline = Date().addingTimeInterval(90)
        while process.isRunning {
            let size = (try? FileManager.default.attributesOfItem(atPath: logURL.path)[.size] as? NSNumber)?.intValue ?? 0
            if cancelled() || Date() >= deadline || size > 1024 * 1024 {
                process.terminate()
                let stop = Date().addingTimeInterval(1)
                while process.isRunning && Date() < stop { Thread.sleep(forTimeInterval: 0.02) }
                if process.isRunning { kill(process.processIdentifier, SIGKILL) }
                process.waitUntilExit()
                throw cancelled() ? OrganizeError.cancelled : OrganizeError.processFailed("编辑引擎超时或输出超限")
            }
            Thread.sleep(forTimeInterval: 0.02)
        }
        guard let size = try? FileManager.default.attributesOfItem(atPath: responseURL.path)[.size] as? NSNumber, size.intValue <= 4 * 1024 * 1024 else { throw OrganizeError.processFailed("本机 PyMuPDF 运行时不可用或响应无效") }
        let response = try Data(contentsOf: responseURL)
        guard process.terminationStatus == 0 else {
            let json = try? JSONSerialization.jsonObject(with: response) as? [String: Any]
            throw OrganizeError.processFailed(String((json?["error"] as? String ?? "内容编辑引擎拒绝该操作").prefix(400)))
        }
        return response
    }
}
