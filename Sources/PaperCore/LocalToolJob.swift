import Foundation

/// Shared cancellation flag. Cancellation never publishes a partial result.
public final class LocalToolCancellation: @unchecked Sendable {
    private let lock = NSLock()
    private var value = false
    public init() {}
    public func cancel() { lock.lock(); value = true; lock.unlock() }
    public var isCancelled: Bool { lock.lock(); defer { lock.unlock() }; return value }
}

public struct LocalToolResult { public let data: Data; public let message: String }

public enum LocalToolJob {
    public static func run(data: Data, root: URL = FileManager.default.temporaryDirectory,
                           cancellation: LocalToolCancellation,
                           operation: @escaping (URL, URL, LocalToolCancellation) throws -> String) async throws -> LocalToolResult {
        try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                do {
                    let result: LocalToolResult = try {
                    guard !cancellation.isCancelled else { throw OrganizeError.cancelled }
                    let dir = root.appendingPathComponent("PDFMaster-job-" + UUID().uuidString)
                    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
                    defer { try? FileManager.default.removeItem(at: dir) }
                    let input = dir.appendingPathComponent("input.pdf"), output = dir.appendingPathComponent("output.pdf")
                    guard FileManager.default.createFile(atPath: input.path, contents: data, attributes: [.posixPermissions: 0o600]) else { throw OrganizeError.invalidInput }
                    let message = try operation(input, output, cancellation)
                    guard !cancellation.isCancelled else { throw OrganizeError.cancelled }
                    let attributes = try FileManager.default.attributesOfItem(atPath: output.path)
                    guard attributes[.type] as? FileAttributeType == .typeRegular,
                          let size = attributes[.size] as? NSNumber, size.intValue <= 256 * 1024 * 1024 else { throw OrganizeError.invalidInput }
                    let result = LocalToolResult(data: try Data(contentsOf: output), message: message)
                    guard !cancellation.isCancelled else { throw OrganizeError.cancelled }
                    return result
                    }()
                    continuation.resume(returning: result)
                } catch { continuation.resume(throwing: error) }
            }
        }
    }
}
