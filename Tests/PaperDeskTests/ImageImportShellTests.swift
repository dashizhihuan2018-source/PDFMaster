import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor final class ImageImportShellTests: XCTestCase {
    private func shell() throws -> ApplicationShellController {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("ImportShell-\(UUID())")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: folder) }
        let app = ApplicationShellController(recentStore: RecentDocumentStore(url: folder.appendingPathComponent("recents.json")))
        _ = app.view // Match the installed window lifecycle before creating tabs.
        return app
    }
    func testGeneratedPDFIsDirtyIndependentAndHasNoSourceOrBlankUndo() throws {
        let app = try shell()
        let original = app.createBlankDocument()
        let before = try XCTUnwrap(original.controller.workspace.document?.dataRepresentation())
        let tab = try app.createImportedDocument(ImageImportOutput(data: before, pageCount: 1, suggestedName: "图片合并.pdf"))
        XCTAssertEqual(app.documentTabs.count, 2)
        XCTAssertTrue(app.activeWorkbench === tab.controller)
        XCTAssertEqual(tab.title, "图片合并.pdf")
        XCTAssertNil(tab.sourceURL)
        XCTAssertNil(tab.controller.workspace.sourceURL)
        XCTAssertTrue(tab.controller.workspace.isDirty)
        XCTAssertFalse(tab.controller.workspace.canUndo)
        XCTAssertEqual(original.controller.workspace.revision, 0)
        XCTAssertFalse(original.controller.workspace.isDirty)
        XCTAssertTrue(app.recentStore.entries.isEmpty)
        XCTAssertFalse(app.close(tabID: tab.id, dirtyDecision: { false }))
    }
    func testInvalidImportedDataCannotAddTabOrReplaceSelection() throws {
        let app = try shell()
        let original = app.createBlankDocument()
        XCTAssertThrowsError(try app.createImportedDocument(ImageImportOutput(data: Data("bad".utf8), pageCount: 1, suggestedName: "bad.pdf")))
        XCTAssertEqual(app.documentTabs.count, 1)
        XCTAssertTrue(app.activeWorkbench === original.controller)
        let valid = try XCTUnwrap(original.controller.workspace.document?.dataRepresentation())
        XCTAssertThrowsError(try app.createImportedDocument(ImageImportOutput(data: valid, pageCount: 2, suggestedName: "bad-count.pdf")))
        XCTAssertEqual(app.documentTabs.count, 1)
    }
    func testImageRouteDoesNotOpenPDFOrReactivatePreviousTab() throws {
        let app = try shell()
        let original = app.createBlankDocument()
        app.showHome()
        var shown: ImageImportController?
        app.imageImportPresenter = { shown = $0 }
        var opened = false
        XCTAssertTrue(app.dispatch(toolID: "image-to-pdf", openProvider: { opened = true; return nil }))
        XCTAssertFalse(opened)
        XCTAssertNil(app.activeWorkbench)
        let importer = try XCTUnwrap(shown)
        let data = try XCTUnwrap(original.controller.workspace.document?.dataRepresentation())
        importer.onImport?(ImageImportOutput(data: data, pageCount: 1, suggestedName: "image.pdf"))
        XCTAssertEqual(app.documentTabs.count, 2)
        XCTAssertEqual(app.activeWorkbench?.workspace.document?.pageCount, 1)
        XCTAssertTrue(app.activeWorkbench?.workspace.isDirty == true)
        XCTAssertNil(app.imageImportController)
    }
    func testDiscardedImporterCannotPublishIntoNewSession() throws {
        let app = try shell()
        let original = app.createBlankDocument()
        app.imageImportPresenter = { _ in }
        XCTAssertTrue(app.dispatch(toolID: "image-to-pdf", openProvider: { nil }))
        let obsolete = try XCTUnwrap(app.imageImportController)
        XCTAssertTrue(app.dispatch(toolID: "image-to-pdf", openProvider: { nil }))
        let data = try XCTUnwrap(original.controller.workspace.document?.dataRepresentation())
        obsolete.onImport?(ImageImportOutput(data: data, pageCount: 1, suggestedName: "old.pdf"))
        XCTAssertEqual(app.documentTabs.count, 1)
        XCTAssertTrue(app.activeWorkbench === original.controller)
    }
}
