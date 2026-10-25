import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor final class ReaderLayoutOptionsTests: XCTestCase {
    func testButtonsSwitchRealPDFViewAndCoverWithoutEditingDocument() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); try c.workspace.insertBlank(at: 1)
        try c.workspace.insertBlank(at: 2); c.refreshDocument()
        let before = try c.workspace.engineInput()
        let document = c.workspace.document
        let revision = c.workspace.revision, dirty = c.workspace.isDirty
        let panel = ReaderLayoutOptionsController(commands: c.readerCommands); _ = panel.view
        XCTAssertEqual(panel.singleButton.state, .on)
        XCTAssertTrue(panel.coverButton.isHidden)
        panel.doubleButton.performClick(nil)
        XCTAssertEqual(c.pdfView.displayMode, .twoUpContinuous)
        XCTAssertFalse(c.pdfView.displaysAsBook)
        XCTAssertEqual(panel.doubleButton.state, .on)
        XCTAssertFalse(panel.coverButton.isHidden)
        panel.coverButton.performClick(nil)
        XCTAssertTrue(c.pdfView.displaysAsBook)
        XCTAssertEqual(c.readerCommands.layout, .twoPagesWithCover)
        panel.coverButton.performClick(nil)
        XCTAssertFalse(c.pdfView.displaysAsBook)
        panel.continuousButton.performClick(nil)
        XCTAssertEqual(c.pdfView.displayMode, .twoUp)
        panel.singleButton.performClick(nil)
        XCTAssertEqual(c.pdfView.displayMode, .singlePage)
        XCTAssertTrue(panel.coverButton.isHidden)
        // PDFKit adds fresh /ID and timestamp fields on serialization; compare
        // the workspace's normalized content contract, not volatile PDF bytes.
        XCTAssertNoThrow(try c.workspace.validateEngineInput(before))
        XCTAssertTrue(c.workspace.document === document)
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertEqual(c.workspace.isDirty, dirty)
    }

    func testPanelRefreshTracksMenuCommandsAndKeepsOtherTabsUnchanged() throws {
        let a = WorkbenchController(), b = WorkbenchController(); _ = a.view; _ = b.view
        for c in [a, b] { try c.workspace.createBlank(); c.refreshDocument() }
        let panel = ReaderLayoutOptionsController(commands: a.readerCommands); _ = panel.view
        XCTAssertTrue(a.readerCommands.run(.cover)); panel.refresh()
        XCTAssertEqual(panel.doubleButton.state, .on)
        XCTAssertEqual(panel.coverButton.state, .on)
        XCTAssertEqual(b.pdfView.displayMode, .singlePageContinuous)
        XCTAssertTrue(a.readerCommands.run(.singlePage)); panel.refresh()
        XCTAssertEqual(panel.singleButton.state, .on)
        XCTAssertTrue(panel.coverButton.isHidden)
    }
}
