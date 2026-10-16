import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor final class DocumentSaveTests: XCTestCase {
    func testNormalSaveHasAnIndependentActionInsteadOfAlwaysSavingACopy() {
        let controller = WorkbenchController()
        XCTAssertTrue(controller.responds(to: NSSelectorFromString("saveDocument")),
                      "Normal Save must not route to Save Copy and ask for a new destination every time")
    }

    func testFirstSaveChoosesOnceThenRepeatedEditsSaveToSameFile() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let url = folder.appendingPathComponent("验收.pdf")
        let controller = WorkbenchController(); _ = controller.view
        try controller.workspace.createBlank(); controller.refreshDocument()
        var choices = 0
        let choose: () -> URL? = { choices += 1; return url }
        XCTAssertTrue(controller.saveDocumentInteractively(chooseDestination: choose))
        for angle in [90, 180, 270] {
            try controller.workspace.rotate(page: 0, degrees: 90)
            XCTAssertTrue(controller.saveDocumentInteractively(chooseDestination: choose))
            XCTAssertEqual(PDFDocument(url: url)?.page(at: 0)?.rotation, angle)
            XCTAssertFalse(controller.workspace.isDirty)
        }
        XCTAssertEqual(choices, 1)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: folder.path), ["验收.pdf"])
    }

    func testCancelFirstSavePreservesUnsavedDocumentAndSaveAsChangesTabIdentity() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let controller = WorkbenchController(); _ = controller.view
        try controller.workspace.createBlank(); try controller.workspace.rotate(page: 0, degrees: 90)
        XCTAssertFalse(controller.saveDocumentInteractively(chooseDestination: { nil }))
        XCTAssertTrue(controller.workspace.isDirty); XCTAssertNil(controller.workspace.sourceURL)
        let tab = DocumentTab(controller: controller, sourceURL: nil)
        let first = folder.appendingPathComponent("first.pdf"), second = folder.appendingPathComponent("second.pdf")
        XCTAssertTrue(controller.saveDocumentInteractively(chooseDestination: { first }))
        let original = try Data(contentsOf: first)
        try controller.workspace.rotate(page: 0, degrees: 90)
        XCTAssertTrue(controller.saveDocumentInteractively(forceChooseDestination: true, chooseDestination: { second }))
        XCTAssertEqual(tab.sourceURL, second); XCTAssertEqual(tab.title, "second.pdf")
        try controller.workspace.rotate(page: 0, degrees: 90)
        XCTAssertTrue(controller.saveDocumentInteractively(chooseDestination: { XCTFail("Must reuse Save As destination"); return nil }))
        XCTAssertEqual(try Data(contentsOf: first), original)
        XCTAssertEqual(PDFDocument(url: second)?.page(at: 0)?.rotation, 270)
    }
}
