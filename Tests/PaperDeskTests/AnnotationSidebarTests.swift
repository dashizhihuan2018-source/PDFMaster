import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor
final class AnnotationSidebarTests: XCTestCase {
    func testAnnotationRowsAreCompactAndRenderRecordedTimeAtNarrowWidths() throws {
        let (sidebar, document) = try fixture()
        let shape = annotation(.square)
        shape.modificationDate = Date(timeIntervalSince1970: 1_798_410_000)
        document.page(at: 0)?.addAnnotation(shape)
        let note = annotation(.text, contents: String(repeating: "Long comment ", count: 30))
        note.modificationDate = nil
        document.page(at: 1)?.addAnnotation(note)
        sidebar.show(.annotations)
        for width: CGFloat in [176, 240, 300] {
            sidebar.view.frame = CGRect(x: 0, y: 0, width: width, height: 700)
            sidebar.view.layoutSubtreeIfNeeded()
            sidebar.table.layoutSubtreeIfNeeded()
            XCTAssertLessThanOrEqual(sidebar.table.rect(ofRow: 0).height, 44)
            XCTAssertLessThanOrEqual(sidebar.table.rect(ofRow: 1).height, 60)
            for row in 0..<2 {
                let cell = try XCTUnwrap(sidebar.table.view(atColumn: 0, row: row, makeIfNecessary: true))
                cell.layoutSubtreeIfNeeded()
                let labels = cell.subviews.compactMap { $0 as? NSTextField }
                let timestamp = try XCTUnwrap(labels.first { $0.identifier?.rawValue == "annotation-time" })
                if row == 0 {
                    let formatter = DateFormatter()
                    formatter.locale = Locale(identifier: "zh_CN")
                    formatter.dateFormat = "yyyy-MM-dd HH:mm"
                    XCTAssertEqual(timestamp.stringValue, formatter.string(from: try XCTUnwrap(shape.modificationDate)))
                } else { XCTAssertEqual(timestamp.stringValue, "时间未记录") }
                XCTAssertGreaterThan(timestamp.frame.height, 9)
                XCTAssertGreaterThanOrEqual(timestamp.frame.minX, 0)
                XCTAssertLessThanOrEqual(timestamp.frame.maxX, cell.bounds.width + 0.5)
                XCTAssertLessThanOrEqual(timestamp.frame.maxY, cell.bounds.height + 0.5)
            }
        }
        XCTAssertNil(note.modificationDate, "Viewing old annotations must not invent a date")
        sidebar.show(.search)
        XCTAssertEqual(sidebar.table.rowHeight, 54, "Other sidebar modes retain their layout")
    }

    func testAnnotationModeAppendsWithoutChangingExistingRawValues() {
        XCTAssertEqual(ReaderSidebarMode.thumbnails.rawValue, 0)
        XCTAssertEqual(ReaderSidebarMode.outline.rawValue, 1)
        XCTAssertEqual(ReaderSidebarMode.search.rawValue, 2)
        XCTAssertNotNil(ReaderSidebarMode(rawValue: 3), "The sidebar needs an A / 批注 mode")
    }

    func testProjectsActualPresentationAnnotationsWithOriginalIndicesAndExcerpts() throws {
        let (sidebar, document) = try fixture()
        let first = try XCTUnwrap(document.page(at: 0))
        first.addAnnotation(annotation(.link))
        let highlight = annotation(.highlight)
        highlight.color = .systemYellow
        first.addAnnotation(highlight)
        first.addAnnotation(annotation(.widget))
        let note = annotation(.text, contents: "Review this detail")
        first.addAnnotation(note)
        first.addAnnotation(annotation(.popup))
        let second = try XCTUnwrap(document.page(at: 1))
        for type in [PDFAnnotationSubtype.ink, .square, .freeText, .underline, .strikeOut] {
            second.addAnnotation(annotation(type, contents: "Marked"))
        }
        sidebar.show(.annotations)
        XCTAssertEqual(sidebar.annotationRows.map(\.pageIndex), [0, 0, 1, 1, 1, 1, 1])
        XCTAssertEqual(sidebar.annotationRows.map(\.annotationIndex), [1, 3, 0, 1, 2, 3, 4])
        XCTAssertEqual(sidebar.annotationRows.first?.type, "Highlight")
        XCTAssertEqual(sidebar.annotationRows.first?.color, .systemYellow)
        XCTAssertTrue(try XCTUnwrap(sidebar.annotationRows.first).excerpt.contains("Sidebar fixture"))
        XCTAssertEqual(sidebar.annotationRows.dropFirst().first?.contents, "Review this detail")
        XCTAssertEqual(sidebar.numberOfRows(in: sidebar.table), 7)
        XCTAssertFalse(sidebar.isAnnotationEmptyStateVisible)
        XCTAssertTrue(sidebar.annotationSummary.contains("Review this detail"))
    }

    func testSelectionNavigatesAndCallsCoordinatorUsingActualIndices() throws {
        let (sidebar, document) = try fixture()
        document.page(at: 1)?.addAnnotation(annotation(.ink))
        sidebar.show(.annotations)
        var selection: (Int, Int)?
        sidebar.onAnnotationSelection = { selection = ($0, $1) }
        XCTAssertTrue(sidebar.navigateToAnnotationRow(0))
        XCTAssertEqual(selection?.0, 1)
        XCTAssertEqual(selection?.1, 0)
        XCTAssertTrue(sidebar.session.pdfView.currentPage === document.page(at: 1))
        XCTAssertEqual(sidebar.selectedAnnotationRow, 0)
        XCTAssertFalse(sidebar.navigateToAnnotationRow(-1))
    }

    func testStaleRowsCannotSelectReplacementDocumentOrShiftedAnnotation() throws {
        let (sidebar, document) = try fixture()
        let page = try XCTUnwrap(document.page(at: 0))
        let old = annotation(.text, contents: "Old")
        page.addAnnotation(old)
        sidebar.show(.annotations)
        var callCount = 0
        sidebar.onAnnotationSelection = { _, _ in callCount += 1 }
        page.removeAnnotation(old)
        page.addAnnotation(annotation(.square))
        XCTAssertFalse(sidebar.navigateToAnnotationRow(0))
        XCTAssertEqual(callCount, 0)
        sidebar.refresh()
        let (_, replacement) = try fixture()
        replacement.page(at: 0)?.addAnnotation(annotation(.text))
        sidebar.session.setDocument(replacement, allowsCopying: true)
        XCTAssertFalse(sidebar.navigateToAnnotationRow(0))
        sidebar.refresh()
        XCTAssertNil(sidebar.selectedAnnotationRow)
        XCTAssertTrue(sidebar.navigateToAnnotationRow(0))
        XCTAssertEqual(callCount, 1)
    }

    func testAnnotationRefreshPreservesCurrentSelectionOnlyAndTracksAddRemove() throws {
        let (sidebar, document) = try fixture()
        let page = try XCTUnwrap(document.page(at: 0))
        let first = annotation(.ink)
        page.addAnnotation(first)
        sidebar.show(.annotations)
        sidebar.session.onChange = { [weak sidebar] in sidebar?.refresh() }
        XCTAssertTrue(sidebar.navigateToAnnotationRow(0))
        let next = annotation(.square)
        page.addAnnotation(next)
        sidebar.session.didUpdateAnnotations(snapshot: document.dataRepresentation())
        XCTAssertEqual(sidebar.annotationRows.count, 2)
        XCTAssertEqual(sidebar.selectedAnnotationRow, 0)
        page.removeAnnotation(first)
        sidebar.session.didUpdateAnnotations(snapshot: document.dataRepresentation())
        XCTAssertEqual(sidebar.annotationRows.count, 1)
        XCTAssertNil(sidebar.selectedAnnotationRow)
        page.removeAnnotation(next)
        sidebar.refresh()
        XCTAssertTrue(sidebar.isAnnotationEmptyStateVisible)
    }

    func testEmptyStateHasWorkingExistingToolsAndFits176PointWidth() throws {
        let (sidebar, _) = try fixture()
        sidebar.show(.annotations)
        XCTAssertEqual(sidebar.annotationEmptyHint, "使用批注和绘画工具标记PDF")
        XCTAssertTrue(sidebar.isAnnotationEmptyStateVisible)
        var activated: [String] = []
        sidebar.onAnnotationTool = { activated.append($0) }
        XCTAssertEqual(sidebar.annotationToolButtons.map(\.toolID), ["highlight", "pen", "text", "note"])
        for button in sidebar.annotationToolButtons { button.performClick(nil) }
        XCTAssertEqual(activated, ["highlight", "pen", "text", "note"])
        sidebar.view.frame = CGRect(x: 0, y: 0, width: 176, height: 620)
        sidebar.view.layoutSubtreeIfNeeded()
        for button in sidebar.annotationToolButtons {
            let rect = sidebar.view.convert(button.bounds, from: button)
            XCTAssertGreaterThanOrEqual(rect.minX, 0)
            XCTAssertLessThanOrEqual(rect.maxX, 176)
        }
    }

    func testSortByModificationDateAndDoNotExtractProtectedHighlightText() throws {
        let (sidebar, document) = try fixture()
        let oldest = annotation(.text, contents: "Oldest")
        oldest.modificationDate = Date(timeIntervalSince1970: 100)
        document.page(at: 0)?.addAnnotation(oldest)
        let latest = annotation(.highlight)
        latest.modificationDate = Date(timeIntervalSince1970: 300)
        document.page(at: 1)?.addAnnotation(latest)
        sidebar.session.setDocument(document, allowsCopying: false)
        sidebar.show(.annotations)
        XCTAssertEqual(sidebar.annotationRows.dropFirst().first?.excerpt, "")
        sidebar.setAnnotationSort(.newestFirst)
        XCTAssertEqual(sidebar.annotationRows.map(\.pageIndex), [1, 0])
        sidebar.setAnnotationSort(.page)
        XCTAssertEqual(sidebar.annotationRows.map(\.pageIndex), [0, 1])
        XCTAssertEqual(sidebar.annotationSummary, "")
    }

    func testDeselectionAndDocumentCloseClearSelectionAndDisableEmptyTools() throws {
        let (sidebar, document) = try fixture()
        document.page(at: 0)?.addAnnotation(annotation(.text))
        sidebar.onAnnotationTool = { _ in }
        sidebar.show(.annotations)
        XCTAssertTrue(sidebar.navigateToAnnotationRow(0))
        sidebar.table.deselectAll(nil)
        sidebar.tableViewSelectionDidChange(Notification(name: NSTableView.selectionDidChangeNotification, object: sidebar.table))
        XCTAssertNil(sidebar.selectedAnnotationRow)
        sidebar.session.setDocument(nil, allowsCopying: false)
        sidebar.refresh()
        XCTAssertTrue(sidebar.annotationRows.isEmpty)
        XCTAssertTrue(sidebar.isAnnotationEmptyStateVisible)
        XCTAssertTrue(sidebar.annotationToolButtons.allSatisfy { !$0.isEnabled })
    }

    private func annotation(_ type: PDFAnnotationSubtype, contents: String? = nil) -> PDFAnnotation {
        let result = PDFAnnotation(bounds: CGRect(x: 30, y: 650, width: 240, height: 40), forType: type, withProperties: nil)
        result.contents = contents
        return result
    }

    private func fixture() throws -> (ReaderSidebarController, PDFDocument) {
        let data = NSMutableData()
        var box = CGRect(x: 0, y: 0, width: 600, height: 800)
        let context = try XCTUnwrap(CGContext(consumer: try XCTUnwrap(CGDataConsumer(data: data)), mediaBox: &box, nil))
        for _ in 0..<2 {
            context.beginPDFPage(nil)
            let line = CTLineCreateWithAttributedString(NSAttributedString(string: "Sidebar fixture selected text", attributes: [.font: NSFont.systemFont(ofSize: 16)]))
            context.textPosition = CGPoint(x: 36, y: 660)
            CTLineDraw(line, context)
            context.endPDFPage()
        }
        context.closePDF()
        let document = try XCTUnwrap(PDFDocument(data: data as Data))
        let session = ReaderSession(pdfView: PDFView(frame: box))
        session.pdfView.displayMode = .singlePage
        session.setDocument(document, allowsCopying: true)
        return (ReaderSidebarController(session: session), document)
    }
}
