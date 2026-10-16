import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor final class PDFSaveTests: XCTestCase {
    private func folder() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("pdf-save-\(UUID())")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }
    func testOpenedFileCanBeSavedRepeatedlyAndReopenedWithoutCopies() throws {
        let directory = try folder(), url = directory.appendingPathComponent("document.pdf")
        let workspace = PDFWorkspace(); try workspace.createBlank(); try workspace.saveCopy(to: url)
        try workspace.open(url)
        for angle in [90, 180, 270] {
            try workspace.rotate(page: 0, degrees: 90); try workspace.save(to: url)
            let reopened = PDFWorkspace(); try reopened.open(url)
            XCTAssertEqual(reopened.document?.page(at: 0)?.rotation, angle)
            XCTAssertEqual(workspace.sourceURL, url); XCTAssertFalse(workspace.isDirty)
        }
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: directory.path), ["document.pdf"])
    }
    func testExternalChangesAreNotOverwrittenAndDirtyEditsRemain() throws {
        let directory = try folder(), url = directory.appendingPathComponent("document.pdf")
        let workspace = PDFWorkspace(); try workspace.createBlank(); try workspace.save(to: url)
        try workspace.rotate(page: 0, degrees: 90)
        let replacement = Data("external change".utf8); try replacement.write(to: url)
        XCTAssertThrowsError(try workspace.save(to: url)) { XCTAssertEqual($0 as? PDFWorkspaceError, .sourceChanged) }
        XCTAssertEqual(try Data(contentsOf: url), replacement); XCTAssertTrue(workspace.isDirty)
        let recovery = directory.appendingPathComponent("recovered.pdf")
        try workspace.save(to: recovery)
        XCTAssertEqual(PDFDocument(url: recovery)?.page(at: 0)?.rotation, 90)
    }
    func testFailedSaveDoesNotAdoptDestinationOrMarkClean() throws {
        let directory = try folder(), url = directory.appendingPathComponent("missing/target.pdf")
        let workspace = PDFWorkspace(); try workspace.createBlank(); try workspace.rotate(page: 0, degrees: 90)
        XCTAssertThrowsError(try workspace.save(to: url))
        XCTAssertNil(workspace.sourceURL); XCTAssertTrue(workspace.isDirty)
    }
    func testSaveAsStillRejectsSourceAliases() throws {
        let directory = try folder(), url = directory.appendingPathComponent("original.pdf")
        let workspace = PDFWorkspace(); try workspace.createBlank(); try workspace.save(to: url)
        let original = try Data(contentsOf: url)
        let alias = directory.appendingPathComponent("alias.pdf")
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: url)
        try workspace.rotate(page: 0, degrees: 90)
        XCTAssertThrowsError(try workspace.save(to: alias))
        XCTAssertEqual(try Data(contentsOf: url), original); XCTAssertTrue(workspace.isDirty)
    }
    func testUndoAfterSaveStillPromptsToSaveChangedDiskContent() throws {
        let url = try folder().appendingPathComponent("undo.pdf")
        let workspace = PDFWorkspace(); try workspace.createBlank(); try workspace.save(to: url)
        try workspace.rotate(page: 0, degrees: 90); try workspace.save(to: url)
        try workspace.undo()
        XCTAssertTrue(workspace.isDirty, "Undo differs from the newly saved file and must not close silently")
        try workspace.save(to: url)
        XCTAssertEqual(PDFDocument(url: url)?.page(at: 0)?.rotation, 0)
    }
    func testEncryptedSourceIsNeverSavedAsUnprotectedOriginal() throws {
        let url = try folder().appendingPathComponent("protected.pdf")
        let workspace = PDFWorkspace(); try workspace.createBlank()
        try workspace.saveEncryptedCopy(to: url, userPassword: "fixture", ownerPassword: "owner-fixture")
        let original = try Data(contentsOf: url)
        try workspace.open(url, password: "fixture")
        XCTAssertThrowsError(try workspace.save(to: url)) { XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied) }
        XCTAssertEqual(try Data(contentsOf: url), original)
    }
}
