import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor final class PageInteractionIntegrationTests: XCTestCase {
    private func controller() throws -> WorkbenchController {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank()
        for index in 1..<4 { try c.workspace.insertBlank(at: index) }
        for index in 0..<4 {
            try c.workspace.addText("PAGE-\(index)", page: index, bounds: CGRect(x: 20, y: 40, width: 180, height: 40))
        }
        c.refreshDocument(); c.enterMode(.pages)
        return c
    }

    func testMainWindowWiresAtomicMultiPageMoveAndSelection() throws {
        let c = try controller()
        let labels = (0..<4).map { c.workspace.document!.page(at: $0)!.annotations.first!.contents }
        let move = try XCTUnwrap(c.pageOverview.onMovePages)
        XCTAssertEqual(move([0, 2], 4), true)
        XCTAssertEqual((0..<4).map { c.workspace.document!.page(at: $0)!.annotations.first!.contents }, [labels[1], labels[3], labels[0], labels[2]])
        XCTAssertEqual(c.pageOverview.selectedPages, [2, 3])
        c.undo()
        XCTAssertEqual((0..<4).map { c.workspace.document!.page(at: $0)!.annotations.first!.contents }, labels)
    }

    func testGridDeleteCallbackDeletesSelectedPagesAndOneUndoRestores() throws {
        let c = try controller()
        c.pageOverview.restoreSelection([1, 3])
        XCTAssertNotNil(c.pageOverview.onDeleteSelection)
        c.pageOverview.onDeleteSelection?()
        XCTAssertEqual(c.workspace.document?.pageCount, 2)
        XCTAssertEqual(c.pageOverview.selectedPages, [1])
        c.undo()
        XCTAssertEqual(c.workspace.document?.pageCount, 4)
    }

    func testSidebarDeleteUsesItsSelectionNotHiddenGridOrCurrentPage() throws {
        let c = try controller()
        c.pageOverview.restoreSelection([0]); c.enterMode(.annotation)
        XCTAssertNotNil(c.readerSidebar.onDeletePages)
        c.readerSidebar.onDeletePages?([1, 3])
        XCTAssertEqual(c.workspace.document?.pageCount, 2)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.contents, "PAGE-0")
        XCTAssertEqual(c.workspace.document?.page(at: 1)?.annotations.first?.contents, "PAGE-2")
        c.undo()
        XCTAssertEqual(c.workspace.document?.pageCount, 4)
    }

    func testDeletingEveryPageIsRejectedWithoutChangingHistory() throws {
        let c = try controller()
        let revision = c.workspace.revision
        XCTAssertThrowsError(try c.deleteSelectedPages([0, 1, 2, 3]))
        XCTAssertEqual(c.workspace.document?.pageCount, 4)
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertThrowsError(try c.deleteSelectedPages([40]))
        XCTAssertEqual(c.workspace.revision, revision)
    }

    func testHiddenSurfaceCallbacksDoNotDeleteVisibleDocumentsPages() throws {
        let c = try controller()
        c.readerSidebar.onDeletePages?([1])
        XCTAssertEqual(c.workspace.document?.pageCount, 4)
        c.enterMode(.annotation)
        c.pageOverview.onDeleteSelection?()
        XCTAssertEqual(c.workspace.document?.pageCount, 4)
    }
}
