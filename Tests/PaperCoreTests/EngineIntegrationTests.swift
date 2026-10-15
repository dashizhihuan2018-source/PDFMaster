import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor
final class EngineIntegrationTests: XCTestCase {
    func testDirectPDFKitMutationCannotBeOverwrittenByEngineResult() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        let token = try workspace.engineInput()
        workspace.document?.page(at: 0)?.rotation = 90
        XCTAssertEqual(workspace.revision, 0)
        XCTAssertThrowsError(try workspace.acceptEngineResult(token.data, for: token)) { XCTAssertEqual($0 as? PDFWorkspaceError, .staleEngineResult) }
        XCTAssertEqual(workspace.document?.page(at: 0)?.rotation, 90)
    }

    func testEngineResultIsUndoableAndKeepsSourceProtection() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let seed = PDFWorkspace(); try seed.createBlank()
        let source = dir.appendingPathComponent("original.pdf")
        try seed.saveCopy(to: source)
        let original = try Data(contentsOf: source)
        let workspace = PDFWorkspace(); try workspace.open(source)
        let candidate = try XCTUnwrap(PDFDocument(data: original))
        candidate.page(at: 0)?.rotation = 90
        let token = try workspace.engineInput()
        try workspace.acceptEngineResult(XCTUnwrap(candidate.dataRepresentation()), for: token)
        XCTAssertEqual(workspace.document?.page(at: 0)?.rotation, 90)
        XCTAssertTrue(workspace.isDirty)
        XCTAssertEqual(workspace.sourceURL, source)
        XCTAssertThrowsError(try workspace.saveCopy(to: source))
        XCTAssertEqual(try Data(contentsOf: source), original)
        workspace.undo()
        XCTAssertEqual(workspace.document?.page(at: 0)?.rotation, 0)
        XCTAssertFalse(workspace.isDirty)
        workspace.redo()
        XCTAssertEqual(workspace.document?.page(at: 0)?.rotation, 90)
    }

    func testEngineResultRejectsStaleRevisionAndDocumentReplacement() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        let token = try workspace.engineInput()
        try workspace.rotate(page: 0, degrees: 90)
        XCTAssertThrowsError(try workspace.acceptEngineResult(token.data, for: token))
        XCTAssertEqual(workspace.document?.page(at: 0)?.rotation, 90)
        try workspace.createBlank() // revision resets to zero but document identity is new
        XCTAssertThrowsError(try workspace.acceptEngineResult(token.data, for: token))
        XCTAssertFalse(workspace.isDirty)
    }

    func testMalformedOrEncryptedOutputDoesNotChangeHistory() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        let token = try workspace.engineInput()
        let original = workspace.document
        XCTAssertThrowsError(try workspace.acceptEngineResult(Data("invalid PDF".utf8), for: token))
        let encrypted = try XCTUnwrap(workspace.document?.dataRepresentation(options: [PDFDocumentWriteOption.userPasswordOption: "fixture", PDFDocumentWriteOption.ownerPasswordOption: "owner-fixture"]))
        XCTAssertThrowsError(try workspace.acceptEngineResult(encrypted, for: token))
        XCTAssertTrue(workspace.document === original)
        XCTAssertFalse(workspace.canUndo)
        XCTAssertFalse(workspace.isDirty)
    }

    func testEncryptedProvenanceCannotEnterUnencryptedEngines() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let seed = PDFWorkspace(); try seed.createBlank()
        let source = dir.appendingPathComponent("encrypted.pdf")
        try seed.saveEncryptedCopy(to: source, userPassword: "fixture", ownerPassword: "owner-fixture")
        let workspace = PDFWorkspace(); try workspace.open(source, password: "fixture")
        XCTAssertTrue(workspace.isEncryptedSource)
        XCTAssertThrowsError(try workspace.engineInput())
        XCTAssertFalse(workspace.isDirty)
    }
}
