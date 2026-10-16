import AppKit
import CoreText
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor final class AnnotationDeletionTests: XCTestCase {
    func testAllThreeMarkupTypesDeleteFromPageWithBothDeleteKeysAndUndoRedo() throws {
        for kind in [PDFAnnotationSubtype.highlight, .underline, .strikeOut] {
            for key: UInt16 in [51, 117] {
                let c = try fixture(kind)
                c.pdfView.onAnnotationSelection?(0, 0)
                c.pdfView.keyDown(with: deletionKey(key))
                XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0, "\(kind) must support Delete")
                XCTAssertEqual(c.pdfView.document?.page(at: 0)?.annotations.count, 0)
                c.undo()
                XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.type, kind.rawValue.trimmingCharacters(in: CharacterSet(charactersIn: "/")))
                c.redo()
                XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
            }
        }
    }

    func testSidebarSelectionDeletesWithKeyboardAndPersistsAfterSaveReopen() throws {
        for kind in [PDFAnnotationSubtype.highlight, .underline, .strikeOut] {
            let c = try fixture(kind)
            c.readerSidebar.show(.annotations)
            XCTAssertTrue(c.readerSidebar.navigateToAnnotationRow(0))
            c.readerSidebar.table.keyDown(with: deletionKey(51))
            XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
            XCTAssertTrue(c.readerSidebar.annotationRows.isEmpty)
            let url = FileManager.default.temporaryDirectory.appendingPathComponent("MarkupDeletion-\(UUID().uuidString).pdf")
            defer { try? FileManager.default.removeItem(at: url) }
            try c.workspace.save(to: url)
            XCTAssertEqual(try XCTUnwrap(PDFDocument(url: url)).page(at: 0)?.annotations.count, 0)
        }
    }

    func testSidebarDeleteButtonIsVisibleEnabledOnlyWithSelectionAndUndoable() throws {
        let c = try fixture(.highlight)
        c.readerSidebar.show(.annotations)
        let button = try XCTUnwrap(descendants(c.readerSidebar.view).compactMap { $0 as? NSButton }
            .first { $0.accessibilityLabel() == "删除所选批注" })
        XCTAssertFalse(button.isEnabled)
        XCTAssertTrue(c.readerSidebar.navigateToAnnotationRow(0))
        XCTAssertTrue(button.isEnabled)
        button.performClick(nil)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        XCTAssertFalse(button.isEnabled)
        c.undo()
        XCTAssertEqual(c.readerSidebar.annotationRows.count, 1)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.type, "Highlight")
    }

    func testStaleSidebarRowAndDeselectedPageCannotDeleteAnotherAnnotation() throws {
        let c = try fixture(.underline)
        c.readerSidebar.show(.annotations)
        XCTAssertTrue(c.readerSidebar.navigateToAnnotationRow(0))
        let displayedPage = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        displayedPage.removeAnnotation(try XCTUnwrap(displayedPage.annotations.first))
        displayedPage.addAnnotation(PDFAnnotation(bounds: CGRect(x: 10, y: 10, width: 40, height: 40), forType: .text, withProperties: nil))
        c.readerSidebar.table.keyDown(with: deletionKey(51))
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.type, "Underline")
        c.pdfView.onShapeDeselection?()
        c.pdfView.keyDown(with: deletionKey(51))
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
    }

    func testNativePageClickAndWindowKeyboardRouteDeleteEachMarkupType() throws {
        for kind in [PDFAnnotationSubtype.highlight, .underline, .strikeOut] {
            let c = try fixture(kind)
            let window = NSWindow(contentRect: c.view.frame, styleMask: [.titled], backing: .buffered, defer: false)
            window.contentViewController = c
            defer { window.contentViewController = nil }
            c.view.layoutSubtreeIfNeeded(); c.pdfView.layoutDocumentView()
            let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
            let bounds = try XCTUnwrap(page.annotations.first).bounds
            let point = c.pdfView.convert(CGPoint(x: bounds.midX, y: bounds.midY), from: page)
            let event = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown,
                location: c.pdfView.convert(point, to: nil), modifierFlags: [], timestamp: 0,
                windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1))
            c.pdfView.mouseDown(with: event)
            XCTAssertTrue(window.firstResponder === c.pdfView)
            window.sendEvent(deletionKey(51, window: window))
            XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        }
    }

    func testTextEditingAndModifiedDeleteNeverRemoveSelectedAnnotation() throws {
        let c = try fixture(.highlight)
        let window = NSWindow(contentRect: c.view.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = c
        defer { window.contentViewController = nil }
        c.pdfView.onAnnotationSelection?(0, 0)
        window.makeFirstResponder(c.pdfView)
        for modifiers: NSEvent.ModifierFlags in [.command, .option, .control, .shift] {
            c.pdfView.keyDown(with: deletionKey(51, window: window, modifiers: modifiers))
            XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
        }
        let field = NSTextField(string: "abc"); field.frame = CGRect(x: 10, y: 10, width: 140, height: 24)
        c.view.addSubview(field); window.makeFirstResponder(field)
        let editor = try XCTUnwrap(window.firstResponder as? NSTextView)
        editor.setSelectedRange(NSRange(location: 3, length: 0))
        window.sendEvent(deletionKey(51, window: window))
        XCTAssertEqual(field.stringValue, "ab")
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
    }

    func testSidebarPermissionPredicateDisablesButtonAndDeleteKey() throws {
        let c = try fixture(.highlight)
        c.readerSidebar.show(.annotations)
        c.readerSidebar.canDeleteAnnotation = { _, _ in false }
        XCTAssertTrue(c.readerSidebar.navigateToAnnotationRow(0))
        let button = try XCTUnwrap(descendants(c.readerSidebar.view).compactMap { $0 as? NSButton }
            .first { $0.accessibilityLabel() == "删除所选批注" })
        XCTAssertFalse(button.isEnabled)
        c.readerSidebar.table.keyDown(with: deletionKey(51))
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
    }

    private func fixture(_ kind: PDFAnnotationSubtype) throws -> WorkbenchController {
        let bytes = NSMutableData()
        var box = CGRect(x: 0, y: 0, width: 600, height: 800)
        let context = try XCTUnwrap(CGContext(consumer: try XCTUnwrap(CGDataConsumer(data: bytes)), mediaBox: &box, nil))
        context.beginPDFPage(nil)
        let line = CTLineCreateWithAttributedString(NSAttributedString(string: "PDF Master markup deletion", attributes: [.font: NSFont.systemFont(ofSize: 24)]))
        context.textPosition = CGPoint(x: 50, y: 650); CTLineDraw(line, context)
        context.endPDFPage(); context.closePDF()
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("MarkupFixture-\(UUID().uuidString).pdf")
        try (bytes as Data).write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }
        let c = WorkbenchController(); _ = c.view
        try c.workspace.open(url)
        let selection = try XCTUnwrap(c.workspace.document?.page(at: 0)?.selection(for: box))
        try c.workspace.addMarkup(selection: selection, kind: kind, color: .systemYellow)
        c.refreshDocument()
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
        return c
    }

    private func deletionKey(_ key: UInt16, window: NSWindow? = nil, modifiers: NSEvent.ModifierFlags = []) -> NSEvent {
        NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: modifiers, timestamp: 0,
                        windowNumber: window?.windowNumber ?? 0, context: nil, characters: "\u{7f}", charactersIgnoringModifiers: "\u{7f}",
                        isARepeat: false, keyCode: key)!
    }
    private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
}
