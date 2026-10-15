import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor
final class ReaderIntegrationTests: XCTestCase {
    func testReaderTabCommandsAreBoundToShell() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: dir) }
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: dir.appendingPathComponent("r.json")))
        _ = shell.view
        let first = shell.createBlankDocument(), second = shell.createBlankDocument()
        shell.activate(tabID: first.id)
        XCTAssertTrue(first.controller.readerCommands.run(.nextTab))
        XCTAssertEqual(shell.activeTabID, second.id)
        XCTAssertTrue(second.controller.readerCommands.run(.previousTab))
        XCTAssertEqual(shell.activeTabID, first.id)
        XCTAssertTrue(first.controller.readerCommands.run(.closeOtherTabs))
        XCTAssertEqual(shell.documentTabs.map(\.id), [first.id])
        XCTAssertFalse(first.controller.readerCommands.run(.closeOtherTabs))
        XCTAssertTrue(first.controller.readerCommands.run(.newTab))
        XCTAssertNil(shell.activeTabID)
    }

    func testLayoutStateFollowsActualPDFView() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument()
        c.pdfView.displayMode = .twoUp; c.pdfView.displaysAsBook = true
        XCTAssertEqual(c.readerCommands.layout, .twoPagesWithCover)
        XCTAssertFalse(c.readerCommands.continuous)
        c.readerCommands.setContinuous(true)
        XCTAssertEqual(c.pdfView.displayMode, .twoUpContinuous)
        XCTAssertTrue(c.pdfView.displaysAsBook)
    }

    func testLargeSearchHasBoundedResultsAndExplicitLimitStatus() async throws {
        let data = NSMutableData()
        var box = CGRect(x: 0, y: 0, width: 600, height: 800)
        let context = try XCTUnwrap(CGContext(consumer: CGDataConsumer(data: data)!, mediaBox: &box, nil))
        for _ in 0..<21 {
            context.beginPDFPage(nil)
            let line = CTLineCreateWithAttributedString(NSAttributedString(string: String(repeating: "a ", count: 50), attributes: [.font: NSFont.systemFont(ofSize: 8)]))
            context.textPosition = CGPoint(x: 20, y: 700); CTLineDraw(line, context)
            context.endPDFPage()
        }
        context.closePDF()
        let reader = ReaderSession(pdfView: PDFView(frame: box))
        reader.setDocument(try XCTUnwrap(PDFDocument(data: data as Data)), allowsCopying: true)
        try reader.search("a")
        for _ in 0..<500 { if !reader.isSearching { break }; try await Task.sleep(nanoseconds: 10_000_000) }
        XCTAssertFalse(reader.isSearching)
        XCTAssertEqual(reader.hits.count, 1000)
        XCTAssertTrue(reader.searchStatus.contains("上限"))
    }

    func testReaderBindsPresentationCopyAndCancelsStateOnRefresh() throws {
        let controller = WorkbenchController(); _ = controller.view
        try controller.workspace.createBlank(); controller.refreshDocument()
        XCTAssertTrue(controller.readerSession.allowsCopying)
        XCTAssertTrue(controller.readerSession.pdfView === controller.pdfView)
        XCTAssertFalse(controller.pdfView.document === controller.workspace.document)
        controller.readerSidebar.show(.outline)
        XCTAssertNotNil(controller.readerSidebar.view.superview)
        try controller.readerSession.search("not found")
        XCTAssertTrue(controller.readerSession.isSearching)
        controller.refreshDocument()
        XCTAssertFalse(controller.readerSession.isSearching)
        XCTAssertTrue(controller.readerSession.hits.isEmpty)
    }

    func testReaderCommandRoutesOnlyToActiveTabAndRevealsSidebar() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: dir) }
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: dir.appendingPathComponent("recents.json")))
        _ = shell.view
        let first = shell.createBlankDocument()
        let second = shell.createBlankDocument()
        XCTAssertTrue(second.controller.readerCommands.run(.twoPages))
        XCTAssertEqual(second.controller.pdfView.displayMode, .twoUpContinuous)
        XCTAssertEqual(first.controller.pdfView.displayMode, .singlePageContinuous)
        XCTAssertTrue(second.controller.readerCommands.run(.toggleSidebar))
        second.controller.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(second.controller.thumbnailSidebarFrame.width, 0, accuracy: 0.1)
        XCTAssertTrue(second.controller.readerCommands.run(.outline))
        second.controller.view.layoutSubtreeIfNeeded()
        XCTAssertGreaterThan(second.controller.thumbnailSidebarFrame.width, 0)
        XCTAssertEqual(second.controller.readerSidebar.mode, .outline)
    }
}
