import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor final class ReaderOutlineSidebarTests: XCTestCase {
    private func document() throws -> PDFDocument {
        let document = PDFDocument()
        for _ in 0..<3 { document.insert(PDFPage(), at: document.pageCount) }
        let root = PDFOutline()
        let chapter = PDFOutline(); chapter.label = "Chapter One"
        chapter.destination = PDFDestination(page: try XCTUnwrap(document.page(at: 0)), at: .zero)
        let nested = PDFOutline(); nested.label = "Nested destination"
        nested.destination = PDFDestination(page: try XCTUnwrap(document.page(at: 2)), at: .zero)
        chapter.insertChild(nested, at: 0); root.insertChild(chapter, at: 0)
        let empty = PDFOutline(); empty.label = " \n\t "
        root.insertChild(empty, at: 1)
        document.outlineRoot = root
        return document
    }

    func testOutlineUsesCompactRowsRatherThanSearchResultWhitespace() throws {
        let session = ReaderSession(pdfView: PDFView())
        session.setDocument(try document(), allowsCopying: true)
        let sidebar = ReaderSidebarController(session: session)
        sidebar.show(.outline)
        XCTAssertEqual(sidebar.table.rowHeight, 28)
        for row in 0..<3 {
            XCTAssertEqual(sidebar.tableView(sidebar.table, heightOfRow: row), 28)
        }
        sidebar.show(.search)
        XCTAssertEqual(sidebar.tableView(sidebar.table, heightOfRow: 0), 54)
    }

    func testWhitespaceOnlyTitlesHaveVisibleFallbackWithoutChangingPDF() throws {
        let pdf = try document()
        let projection = ReaderOutlineItem.project(pdf)
        XCTAssertEqual(projection.map(\.title), ["Chapter One", "Nested destination", "未命名"])
        XCTAssertEqual(pdf.outlineRoot?.child(at: 1)?.label, " \n\t ")
        XCTAssertEqual(projection.map(\.depth), [0, 1, 0])
        XCTAssertNil(projection[2].destination)
    }

    func testOutlineRemainsVisibleAcrossModesRefreshResizeAndDocumentReplacement() throws {
        let session = ReaderSession(pdfView: PDFView())
        let sidebar = ReaderSidebarController(session: session)
        session.onChange = { [weak sidebar] in sidebar?.refresh() }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 176, height: 700),
                              styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.contentViewController = sidebar
        defer { window.contentViewController = nil }
        session.setDocument(try document(), allowsCopying: true)
        for width: CGFloat in [176, 320, 480, 176] {
            window.setContentSize(NSSize(width: width, height: 700))
            for mode: ReaderSidebarMode in [.thumbnails, .annotations, .search, .outline] {
                sidebar.show(mode); sidebar.view.layoutSubtreeIfNeeded()
            }
            sidebar.refresh(); sidebar.view.layoutSubtreeIfNeeded()
            XCTAssertEqual(sidebar.table.numberOfRows, 3)
            XCTAssertFalse(sidebar.table.isHiddenOrHasHiddenAncestor)
            XCTAssertGreaterThan(sidebar.table.visibleRect.height, 80)
            XCTAssertGreaterThan(sidebar.table.tableColumns[0].width, 40)
            let cell = try XCTUnwrap(sidebar.table.view(atColumn: 0, row: 0, makeIfNecessary: true))
            XCTAssertGreaterThan(cell.bounds.width, 40)
            XCTAssertFalse(cell.isHiddenOrHasHiddenAncestor)
        }
        sidebar.navigateToOutlineRow(1)
        XCTAssertTrue(session.pdfView.currentPage === session.pdfView.document?.page(at: 2))
        session.setDocument(PDFDocument(), allowsCopying: true)
        XCTAssertEqual(sidebar.table.numberOfRows, 0)
        session.setDocument(try document(), allowsCopying: true)
        XCTAssertEqual(sidebar.table.numberOfRows, 3)
        XCTAssertEqual(sidebar.outlineRows[0].title, "Chapter One")
    }
}
