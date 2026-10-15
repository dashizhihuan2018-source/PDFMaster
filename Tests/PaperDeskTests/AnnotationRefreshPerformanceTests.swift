import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor final class AnnotationRefreshPerformanceTests: XCTestCase {
    func testEraserKeepsReaderIdentityAndSidebarSelectionCannotEraseInk() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation)
        c.toggleInk(NSMenuItem())
        let document = try XCTUnwrap(c.pdfView.document), page = try XCTUnwrap(document.page(at: 0))
        c.pdfView.beginGesture(on: page, at: CGPoint(x: 70,y: 100)); c.pdfView.updateGesture(to: CGPoint(x: 170,y: 180)); c.pdfView.finishGesture()
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        let eraser = try XCTUnwrap(descendants(c.view).compactMap { $0 as? ToolStripButton }.first { $0.toolID == "eraser" })
        _ = eraser.sendAction(eraser.action, to: eraser.target)
        c.readerSidebar.show(.annotations)
        XCTAssertTrue(c.readerSidebar.navigateToAnnotationRow(0))
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1, "Sidebar navigation is never an eraser command")
        _ = eraser.sendAction(eraser.action, to: eraser.target)
        c.pdfView.onAnnotationSelection?(0,0)
        XCTAssertTrue(c.pdfView.document === document)
        XCTAssertTrue(c.pdfView.document?.page(at: 0) === page)
        XCTAssertEqual(page.annotations.count, 0)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        c.undo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
    }
    func testGridPasteDoesNotRebindHiddenReaderAndOpeningInsertedPageRefreshesOnce() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); try c.workspace.insertBlank(at: 1)
        c.refreshDocument(); c.enterMode(.pages)
        let hidden = c.pdfView.document
        c.pageOverview.restoreSelection([0]); try c.performOverviewAction("copy-page")
        try c.performOverviewAction("paste-page")
        XCTAssertEqual(c.workspace.document?.pageCount, 3)
        XCTAssertTrue(c.pdfView.document === hidden, "A hidden PDFView should not discard/rebuild all page tiles while the grid is active")
        c.pageOverview.openPage(at: 1)
        XCTAssertFalse(c.isOverviewVisible)
        XCTAssertEqual(c.pdfView.document?.pageCount, 3)
        XCTAssertEqual(c.pageBadgeText, "2 / 3")
    }
    func testShapeCreationDeletionAndStyleKeepPresentationAndViewport() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation); c.addRectangle()
        c.pdfView.scaleFactor = 3.66
        let document = try XCTUnwrap(c.pdfView.document), page = try XCTUnwrap(document.page(at: 0))
        c.pdfView.beginGesture(on: page, at: CGPoint(x: 100,y: 150))
        c.pdfView.updateGesture(to: CGPoint(x: 220,y: 230)); c.pdfView.finishGesture()
        XCTAssertTrue(c.pdfView.document === document)
        XCTAssertTrue(c.pdfView.document?.page(at: 0) === page)
        XCTAssertEqual(page.annotations.count, 1)
        c.pdfView.onAnnotationSelection?(0,0)
        c.changeShapeStyle(AnnotationStyle(strokeColor: .blue, fillColor: nil, lineWidth: 8))
        XCTAssertTrue(c.pdfView.document === document)
        XCTAssertEqual(page.annotations.first?.border?.lineWidth, 8)
        c.pdfView.onDeleteShape?()
        XCTAssertTrue(c.pdfView.document === document)
        XCTAssertEqual(page.annotations.count, 0)
        XCTAssertEqual(c.pdfView.scaleFactor, 3.66, accuracy: 0.01)
        c.undo()
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
    }
}
