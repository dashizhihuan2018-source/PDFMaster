import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

final class CanvasShellTests: XCTestCase {
    private func directory() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("canvas-shell-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }
    @MainActor func testCanvasAndPDFTabsStayIndependentAndCloseCancellationProtectsBoard() throws {
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: try directory().appendingPathComponent("recents.json")))
        _ = shell.view
        let pdf = shell.createBlankDocument()
        let canvas = shell.createCanvas()
        XCTAssertTrue(shell.activeCanvas === canvas.controller)
        XCTAssertNil(shell.activeWorkbench)
        try canvas.controller.session.mutate { $0.elements.append(CanvasElement(kind: .sticky, frame: CGRect(x: 20, y: 20, width: 160, height: 160), text: "协同思考")) }
        XCTAssertFalse(shell.close(tabID: canvas.id, dirtyDecision: { false }))
        XCTAssertEqual(shell.canvasTabs.count, 1)
        XCTAssertFalse(shell.guardAllDirty(canvasDecision: { _ in false }, { _ in true }))
        shell.activate(tabID: pdf.id)
        XCTAssertNil(shell.activeCanvas)
        XCTAssertTrue(shell.activeWorkbench === pdf.controller)
        XCTAssertEqual(canvas.controller.session.board.elements.count, 1)
        XCTAssertTrue(shell.close(tabID: canvas.id, dirtyDecision: { true }))
        XCTAssertEqual(shell.documentTabs.count, 1)
    }
    @MainActor func testBoardFileAdmissionDedupeAndInvalidAdmissionAreAtomic() throws {
        let dir = try directory()
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: dir.appendingPathComponent("r.json")))
        let url = dir.appendingPathComponent("ideas.pdfmasterboard")
        try CanvasBoard(title: "设计画布").save(to: url)
        XCTAssertTrue(shell.open(url: url, passwordPrompt: { XCTFail("画布不应弹 PDF 密码框"); return nil }))
        let original = try XCTUnwrap(shell.activeCanvas)
        XCTAssertTrue(shell.openCanvas(url: url))
        XCTAssertTrue(shell.activeCanvas === original)
        XCTAssertEqual(shell.canvasTabs.count, 1)
        let bad = dir.appendingPathComponent("bad.pdfmasterboard")
        try Data("not a board".utf8).write(to: bad)
        XCTAssertFalse(shell.openCanvas(url: bad))
        XCTAssertNotNil(shell.lastOpenError)
        XCTAssertTrue(shell.activeCanvas === original)
        XCTAssertEqual(shell.recentStore.recents.count, 0, "画布不污染 PDF 最近记录")
        XCTAssertFalse(shell.openCanvas(url: URL(string: "https://example.com/remote.pdfmasterboard")!))
    }
    @MainActor func testEmbeddedPDFBridgeUsesIndependentCopyAndDoesNotWriteOriginal() throws {
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: try directory().appendingPathComponent("r.json")))
        let pdf = shell.createBlankDocument()
        try pdf.controller.workspace.addPopupNote("原有批注需保留", author: nil, page: 0, bounds: CGRect(x: 50, y: 50, width: 24, height: 24), color: .yellow)
        let canvas = try shell.sendActivePDFToCanvas()
        let card = try XCTUnwrap(canvas.controller.session.board.elements.first)
        // Independently serialized PDFKit documents may have different document IDs.
        // Freeze the actual embedded bytes and prove copy editing never changes them.
        let original = try XCTUnwrap(card.assetData)
        XCTAssertEqual(card.kind, .pdf)
        XCTAssertTrue(canvas.controller.session.isDirty)
        let copied = try shell.openCanvasPDF(data: try XCTUnwrap(card.assetData), name: "test.pdf")
        XCTAssertTrue(shell.activeWorkbench === copied.controller)
        XCTAssertNil(copied.sourceURL)
        try copied.controller.workspace.addPopupNote("只修改副本", author: nil, page: 0, bounds: CGRect(x: 30, y: 30, width: 20, height: 20), color: .yellow)
        XCTAssertEqual(canvas.controller.session.board.elements.first?.assetData, original)
        XCTAssertEqual(pdf.controller.workspace.document?.page(at: 0)?.annotations.count, 1)
        XCTAssertEqual(copied.controller.workspace.document?.page(at: 0)?.annotations.count, 2)
        XCTAssertEqual(shell.documentTabs.count, 2)
        let activeID = shell.activeTabID
        XCTAssertThrowsError(try shell.openCanvasPDF(data: Data("bad".utf8), name: "bad.pdf"))
        XCTAssertEqual(shell.activeTabID, activeID)
        XCTAssertEqual(shell.documentTabs.count, 2)
    }
    @MainActor func testCanvasDashboardDoesNotReplaceExistingPDFTools() throws {
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: try directory().appendingPathComponent("r.json")))
        shell.view.frame = CGRect(x: 0, y: 0, width: 1280, height: 820)
        shell.dashboardController.select(.canvas)
        shell.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(shell.dashboardController.selectedSection, .canvas)
        XCTAssertTrue(HomeSection.allCases.contains(.canvas))
        XCTAssertEqual(ToolCatalog.all.count, 28)
        shell.dashboardController.select(.home)
        XCTAssertEqual(shell.dashboardController.selectedSection, .home)
        XCTAssertTrue(shell.dashboardController.homeOpenButtonHasAction)
    }
}
