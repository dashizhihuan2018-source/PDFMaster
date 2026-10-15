import Foundation
import XCTest
@testable import PaperCore

final class ExternalToolDiscoveryTests: XCTestCase {
    func testMissingExternalEngineDoesNotUseDeveloperWorkspace() {
        XCTAssertNil(ExternalToolDiscovery.qpdf(environment: ["PATH": "/nonexistent"]))
    }
    func testExplicitExecutableAndPathDiscovery() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let binary = directory.appendingPathComponent("qpdf")
        try Data("#!/bin/sh\nexit 0\n".utf8).write(to: binary)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: binary.path)
        XCTAssertEqual(ExternalToolDiscovery.qpdf(environment: ["PDFMASTER_QPDF": binary.path, "PATH": ""]), binary)
        XCTAssertEqual(ExternalToolDiscovery.qpdf(environment: ["PATH": directory.path]), binary)
        XCTAssertNil(ExternalToolDiscovery.qpdf(environment: ["PDFMASTER_QPDF": "relative/qpdf", "PATH": ""]))
    }
}
