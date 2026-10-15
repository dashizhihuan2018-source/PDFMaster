import XCTest
@testable import PaperCore

final class LocalToolJobTests: XCTestCase {
    func testCancelledJobCannotPublishAndRemovesScratch() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let cancellation = LocalToolCancellation(); cancellation.cancel()
        do {
            _ = try await LocalToolJob.run(data: Data("private".utf8), root: root, cancellation: cancellation) { _, _, _ in
                XCTFail("cancelled work must not execute"); return "unexpected"
            }
            XCTFail("expected cancellation")
        } catch { XCTAssertEqual(error as? OrganizeError, .cancelled) }
        XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: root.path).isEmpty)
    }

    func testJobRunsOffMainThreadAndCleansItsPrivateFiles() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let result = try await LocalToolJob.run(data: Data("snapshot".utf8), root: root, cancellation: LocalToolCancellation()) { input, output, _ in
            XCTAssertFalse(Thread.isMainThread)
            XCTAssertEqual(try Data(contentsOf: input), Data("snapshot".utf8))
            let attrs = try FileManager.default.attributesOfItem(atPath: input.path)
            XCTAssertEqual((attrs[.posixPermissions] as? NSNumber)?.intValue, 0o600)
            try Data("result".utf8).write(to: output)
            return "finished"
        }
        XCTAssertEqual(result.data, Data("result".utf8))
        XCTAssertEqual(result.message, "finished")
        XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: root.path).isEmpty)
    }
}
