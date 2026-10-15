import XCTest
import AppKit
@testable import PaperDesk

@MainActor final class ReferenceToolbarTests: XCTestCase {
    func testFourSquareButtonTogglesFullPageOverviewAndOpensSelectedPage() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); try c.workspace.insertBlank(at: 1); c.refreshDocument()
        c.togglePageOverview()
        XCTAssertTrue(c.isOverviewVisible)
        XCTAssertTrue(c.pdfView.isHidden)
        XCTAssertEqual(c.activeMode, .pages)
        XCTAssertEqual(c.pageOverview.collectionView(c.pageOverview.collectionView, numberOfItemsInSection: 0), 2)
        c.pageOverview.openPage(at: 1)
        XCTAssertFalse(c.isOverviewVisible)
        XCTAssertFalse(c.pdfView.isHidden)
        XCTAssertEqual(c.pdfView.document?.index(for: c.pdfView.currentPage!), 1)
    }

    func testAnnotationAndEditingHaveDistinctCenteredSecondRows() throws {
        let c = WorkbenchController(); _ = c.view; try c.workspace.createBlank(); c.refreshDocument()
        c.enterMode(.annotation)
        XCTAssertEqual(c.secondaryToolIDs, ["highlight", "underline", "strikeout", "pen", "eraser", "text", "shapes", "note", "stamp", "signature", "selection"])
        c.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(c.secondaryToolFrame.midX, c.view.bounds.midX, accuracy: 2)
        c.enterMode(.editing)
        XCTAssertEqual(c.secondaryToolIDs, ["content-text", "content-image", "content-link", "content-redact"])
        XCTAssertFalse(c.pdfView.inkEnabled)
    }

    func testChangingModeCancelsOldDrawingAndPlacement() throws {
        let c = WorkbenchController(); _ = c.view; try c.workspace.createBlank(); c.refreshDocument()
        c.enterMode(.drawing); c.pdfView.inkEnabled = true
        c.enterMode(.annotation)
        XCTAssertFalse(c.pdfView.inkEnabled)
        XCTAssertFalse(c.pdfView.placementEnabled)
    }

    func testOverviewActionsUseGridSelectionAndPreserveItOnRefresh() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank()
        try c.workspace.insertBlank(at: 1); try c.workspace.insertBlank(at: 2)
        c.refreshDocument(); c.togglePageOverview()
        c.pageOverview.restoreSelection(IndexSet([1, 2]))
        try c.performOverviewAction("rotate")
        XCTAssertEqual((0..<3).map { c.workspace.document!.page(at: $0)!.rotation }, [0, 90, 90])
        XCTAssertEqual(c.pageOverview.selectedPages, IndexSet([1, 2]))
        c.undo()
        XCTAssertEqual((0..<3).map { c.workspace.document!.page(at: $0)!.rotation }, [0, 0, 0])
        XCTAssertEqual(c.pageOverview.selectedPages, IndexSet([1, 2]))
        try c.performOverviewAction("copy-page")
        XCTAssertEqual(c.workspace.document?.pageCount, 3, "Copy must not duplicate pages")
        try c.performOverviewAction("paste-page")
        XCTAssertEqual(c.workspace.document?.pageCount, 5)
        XCTAssertEqual(c.pageOverview.selectedPages, IndexSet([3, 4]))
        try c.performOverviewAction("delete-page")
        XCTAssertEqual(c.workspace.document?.pageCount, 3)
        c.undo(); XCTAssertEqual(c.workspace.document?.pageCount, 5)
    }

    func testOverviewRequiresASelectionAndInsertsAfterSelectedPage() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); try c.workspace.insertBlank(at: 1)
        c.refreshDocument(); c.togglePageOverview(); c.pageOverview.restoreSelection([])
        XCTAssertThrowsError(try c.performOverviewAction("rotate"))
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.rotation, 0)
        c.pageOverview.restoreSelection(IndexSet(integer: 1))
        try c.performOverviewAction("insert")
        XCTAssertEqual(c.workspace.document?.pageCount, 3)
        XCTAssertEqual(c.pageOverview.selectedPages, IndexSet(integer: 2))
    }

    func testMenuZoomTargetsOverviewInsteadOfHiddenReader() throws {
        let c = WorkbenchController(); _ = c.view; try c.workspace.createBlank(); c.refreshDocument()
        c.togglePageOverview(); let original = c.pdfView.scaleFactor
        XCTAssertTrue(c.readerCommands.run(.zoomIn))
        XCTAssertEqual(c.pageOverview.thumbnailScale, 1.2, accuracy: 0.01)
        XCTAssertEqual(c.pdfView.scaleFactor, original)
        XCTAssertTrue(c.readerCommands.run(.fitPage))
        XCTAssertEqual(c.pageOverview.thumbnailScale, 1)
        XCTAssertFalse(c.readerCommands.run(.twoPages))
    }
}
