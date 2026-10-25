import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor final class PageDeletionToolbarTests: XCTestCase {
    private func controller() throws -> WorkbenchController {
        _ = NSApplication.shared
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank()
        for index in 1..<4 { try c.workspace.insertBlank(at: index) }
        for index in 0..<4 { try c.workspace.addText("PAGE-\(index)", page: index, bounds: CGRect(x: 20, y: 40, width: 150, height: 30)) }
        try c.workspace.addBookmark(label: "Bookmarked page", page: 1)
        c.refreshDocument(); c.enterMode(.pages)
        c.pageOverview.restoreSelection([1])
        return c
    }

    private func tools(in view: NSView) -> [ToolStripButton] {
        (view as? ToolStripButton).map { [$0] } ?? view.subviews.flatMap { tools(in: $0) }
    }

    // Exercise the actual NSButton target/action and NSAlert modal completion,
    // rather than bypassing the toolbar via performOverviewAction in the test.
    private func clickDelete(_ c: WorkbenchController, confirm: Bool) throws -> Int {
        let button = try XCTUnwrap(tools(in: c.view).first { $0.toolID == "delete-page" })
        var alerts = 0
        var handledWindows: [NSWindow] = []
        let timer = Timer(timeInterval: 0.01, repeats: true) { _ in
            MainActor.assumeIsolated {
                guard let window = NSApp.modalWindow,
                      !handledWindows.contains(where: { $0 === window }) else { return }
                // A closing alert can remain modal for another timer tick.
                // Never answer the same confirmation a second time as Cancel.
                handledWindows.append(window)
                alerts += 1
                NSApp.stopModal(withCode: alerts == 1 && confirm ? .alertFirstButtonReturn : .alertSecondButtonReturn)
            }
        }
        RunLoop.main.add(timer, forMode: .modalPanel)
        defer { timer.invalidate() }
        button.performClick(nil)
        return alerts
    }

    func testToolbarDeletesBookmarkedGridSelectionAndUndoRedoRestoresIt() throws {
        let c = try controller()
        XCTAssertEqual(try clickDelete(c, confirm: true), 1, "A valid bookmarked page must not produce an error alert")
        XCTAssertEqual(c.workspace.document?.pageCount, 3)
        XCTAssertEqual(c.workspace.document?.page(at: 1)?.annotations.first?.contents, "PAGE-2")
        XCTAssertEqual(c.pageOverview.selectedPages, [1])
        c.undo()
        XCTAssertEqual(c.workspace.document?.pageCount, 4)
        XCTAssertEqual(c.workspace.document?.outlineRoot?.child(at: 0)?.label, "Bookmarked page")
        c.redo(); XCTAssertEqual(c.workspace.document?.pageCount, 3)
    }

    func testCancelDoesNotDeleteOrConsumeHistory() throws {
        let c = try controller(), revision = c.workspace.revision
        XCTAssertEqual(try clickDelete(c, confirm: false), 1)
        XCTAssertEqual(c.workspace.document?.pageCount, 4)
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertEqual(c.pageOverview.selectedPages, [1])
    }
}
