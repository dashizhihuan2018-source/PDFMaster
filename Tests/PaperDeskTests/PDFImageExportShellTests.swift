import AppKit
import XCTest
@testable import PaperDesk

@MainActor final class PDFImageExportShellTests: XCTestCase {
    func testCatalogExposesPDFImageExportAsPartialRealWorkflow() throws {
        let descriptor = try XCTUnwrap(ToolCatalog.descriptor(id: "pdf-to-image"))
        XCTAssertEqual(descriptor.availability, .partial)
    }

    func testRoutePresentsExporterForActiveDocumentWithoutMutatingIt() throws {
        let folder = temporaryDirectory()
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: folder.appendingPathComponent("recents.json")))
        _ = shell.view
        let tab = shell.createBlankDocument()
        let revision = tab.controller.workspace.revision
        shell.showHome()
        var shown: PDFImageExportController?
        shell.pdfImageExportPresenter = { shown = $0 }

        XCTAssertTrue(shell.dispatch(toolID: "pdf-to-image", openProvider: { nil }))
        XCTAssertTrue(shell.activeWorkbench === tab.controller)
        XCTAssertNotNil(shown)
        XCTAssertTrue(shell.pdfImageExportController === shown)
        XCTAssertEqual(tab.controller.workspace.revision, revision)
        XCTAssertFalse(tab.controller.workspace.isDirty)
    }

    func testRouteWithoutDocumentOrPresenterCannotClaimStarted() throws {
        let folder = temporaryDirectory()
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: folder.appendingPathComponent("recents.json")))
        _ = shell.view
        XCTAssertFalse(shell.dispatch(toolID: "pdf-to-image", openProvider: { nil }))
    }

    private func temporaryDirectory() -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("PDFImageExportShellTests-\(UUID().uuidString)", isDirectory: true)
        try! FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }
}
