import AppKit
import CoreText
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor final class MarkupToolFirstTests: XCTestCase {
    func testToolFirstArmsWithoutAlertAndCommitsOnlyOnMouseReleaseForAllThreeTools() throws {
        for (id, type) in [("highlight", "Highlight"), ("underline", "Underline"), ("strikeout", "StrikeOut")] {
            let (c, window) = try fixture()
            defer { window.contentViewController = nil }
            let tool = try button(id, c)
            activate(tool)
            XCTAssertEqual(tool.state, .on, "Clicking a markup tool before selecting text must arm it")
            guard isArmed(c) else { continue }
            let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
            let start = page.characterBounds(at: 0), end = page.characterBounds(at: 4)
            send(.leftMouseDown, CGPoint(x: start.midX, y: start.midY), page, c)
            send(.leftMouseDragged, CGPoint(x: end.midX, y: end.midY), page, c)
            XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
            XCTAssertFalse(c.pdfView.currentSelection?.string?.isEmpty ?? true)
            send(.leftMouseUp, CGPoint(x: end.midX, y: end.midY), page, c)
            XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.type, type)
            XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
            XCTAssertNil(c.pdfView.currentSelection)
            XCTAssertEqual(tool.state, .on, "The tool stays active for the next selection")
        }
    }

    func testToolFirstSupportsRepeatedSelectionsSwitchingAndUndoSaveReopen() throws {
        let (c, window) = try fixture()
        defer { window.contentViewController = nil }
        let highlight = try button("highlight", c)
        activate(highlight)
        XCTAssertEqual(highlight.state, .on)
        guard isArmed(c) else { return }
        try drag(c, from: 0, to: 4)
        try drag(c, from: 6, to: 11)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.map(\.type), ["Highlight", "Highlight"])
        let underline = try button("underline", c)
        activate(underline)
        XCTAssertEqual(highlight.state, .off); XCTAssertEqual(underline.state, .on)
        try drag(c, from: 0, to: 4)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.last?.type, "Underline")
        c.undo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 2)
        c.redo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 3)
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("ToolFirstSaved-\(UUID().uuidString).pdf")
        defer { try? FileManager.default.removeItem(at: url) }
        try c.workspace.save(to: url)
        XCTAssertEqual(PDFDocument(url: url)?.page(at: 0)?.annotations.map(\.type), ["Highlight", "Highlight", "Underline"])
    }

    func testTextFirstStillAppliesImmediatelyWithoutRepeatingTheSameSelection() throws {
        let (c, window) = try fixture()
        defer { window.contentViewController = nil }
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        c.pdfView.setCurrentSelection(page.selection(for: NSRange(location: 0, length: 5)), animate: false)
        activate(try button("strikeout", c))
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.type, "StrikeOut")
        XCTAssertNil(c.pdfView.currentSelection)
        activate(try button("underline", c))
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
    }

    func testEscapeModeSwitchAndOtherToolsCancelPendingMarkup() throws {
        let (c, window) = try fixture()
        defer { window.contentViewController = nil }
        let tool = try button("highlight", c)
        activate(tool); XCTAssertEqual(tool.state, .on)
        guard isArmed(c) else { return }
        XCTAssertTrue(c.pdfView.cancelPlacementForEscape())
        XCTAssertEqual(tool.state, .off)
        activate(tool); c.addRectangle()
        XCTAssertEqual(tool.state, .off); XCTAssertTrue(c.pdfView.placementEnabled)
        activate(tool)
        XCTAssertFalse(c.pdfView.placementEnabled); XCTAssertFalse(c.pdfView.inkEnabled)
        c.enterMode(.editing)
        XCTAssertFalse(c.pdfView.cancelPlacementForEscape())
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
    }

    func testBlankDragAndClickDoNotCreateMarkupAndMultilineDragIsOneUndoStep() throws {
        let (c, window) = try fixture()
        defer { window.contentViewController = nil }
        let tool = try button("highlight", c)
        activate(tool); XCTAssertEqual(tool.state, .on)
        guard isArmed(c) else { return }
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        for (start, end) in [(CGPoint(x: 400, y: 250), CGPoint(x: 500, y: 300)),
                             (CGPoint(x: 52, y: 654), CGPoint(x: 52, y: 654))] {
            send(.leftMouseDown, start, page, c); send(.leftMouseDragged, end, page, c); send(.leftMouseUp, end, page, c)
        }
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        let lastIndex = try XCTUnwrap((0..<page.numberOfCharacters).reversed().first { !page.characterBounds(at: $0).isEmpty })
        let first = page.characterBounds(at: 0), last = page.characterBounds(at: lastIndex)
        send(.leftMouseDown, CGPoint(x: first.midX, y: first.midY), page, c)
        send(.leftMouseDragged, CGPoint(x: last.midX, y: last.midY), page, c)
        XCTAssertEqual(c.pdfView.currentSelection?.selectionsByLine().count, 2)
        send(.leftMouseUp, CGPoint(x: last.midX, y: last.midY), page, c)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 2)
        c.undo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        c.redo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 2)
    }

    func testReverseDragAtZoomAndRotationUsesPageCoordinates() throws {
        for rotation in [0, 90, 180, 270] {
            let (c, window) = try fixture()
            defer { window.contentViewController = nil }
            c.workspace.document?.page(at: 0)?.rotation = rotation
            c.refreshDocument()
            activate(try button("underline", c))
            guard isArmed(c) else { continue }
            let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
            c.view.layoutSubtreeIfNeeded()
            c.pdfView.autoScales = false; c.pdfView.scaleFactor = 0.65; c.pdfView.layoutDocumentView()
            c.pdfView.go(to: try XCTUnwrap(page.selection(for: NSRange(location: 0, length: 11))))
            let a = page.characterBounds(at: 10), b = page.characterBounds(at: 0)
            send(.leftMouseDown, CGPoint(x:a.midX,y:a.midY), page, c)
            send(.leftMouseDragged, CGPoint(x:b.midX,y:b.midY), page, c)
            XCTAssertNotNil(c.pdfView.currentSelection, "Reverse selection at rotation \(rotation)")
            send(.leftMouseUp, CGPoint(x:b.midX,y:b.midY), page, c)
            XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.type, "Underline", "rotation \(rotation)")
            XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
        }
    }

    func testCrossPageDragIsOneUndoStepAndReplacingDocumentCancelsGesture() throws {
        let (c, window) = try fixture(twoPages: true)
        defer { window.contentViewController = nil }
        c.pdfView.autoScales = false; c.pdfView.scaleFactor = 0.3; c.pdfView.layoutDocumentView()
        activate(try button("highlight", c))
        guard isArmed(c) else { return }
        let firstPage = try XCTUnwrap(c.pdfView.document?.page(at: 0)), secondPage = try XCTUnwrap(c.pdfView.document?.page(at: 1))
        let a = firstPage.characterBounds(at: 0), b = secondPage.characterBounds(at: 4)
        send(.leftMouseDown, CGPoint(x: a.midX, y: a.midY), firstPage, c)
        send(.leftMouseDragged, CGPoint(x: b.midX, y: b.midY), secondPage, c)
        send(.leftMouseUp, CGPoint(x: b.midX, y: b.midY), secondPage, c)
        XCTAssertGreaterThan(c.workspace.document?.page(at: 0)?.annotations.count ?? 0, 0)
        XCTAssertGreaterThan(c.workspace.document?.page(at: 1)?.annotations.count ?? 0, 0)
        c.undo()
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        XCTAssertEqual(c.workspace.document?.page(at: 1)?.annotations.count, 0)
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        send(.leftMouseDown, CGPoint(x: a.midX, y: a.midY), page, c)
        c.refreshDocument()
        send(.leftMouseUp, CGPoint(x: b.midX, y: b.midY), try XCTUnwrap(c.pdfView.document?.page(at: 1)), c)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        XCTAssertEqual(c.workspace.document?.page(at: 1)?.annotations.count, 0)
    }

    private func fixture(twoPages: Bool = false) throws -> (WorkbenchController, NSWindow) {
        _ = NSApplication.shared
        let data = NSMutableData(); var box = CGRect(x: 0, y: 0, width: 600, height: 800)
        let cg = try XCTUnwrap(CGContext(consumer: try XCTUnwrap(CGDataConsumer(data: data)), mediaBox: &box, nil))
        for _ in 0..<(twoPages ? 2 : 1) {
            cg.beginPDFPage(nil)
            for (text, y) in [("First markup selection", 650.0), ("Second markup selection", 600.0)] {
                let line = CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: [.font: NSFont.systemFont(ofSize: 24)]))
                cg.textPosition = CGPoint(x: 50, y: y); CTLineDraw(line, cg)
            }
            cg.endPDFPage()
        }
        cg.closePDF()
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("ToolFirst-\(UUID().uuidString).pdf")
        try (data as Data).write(to: url); defer { try? FileManager.default.removeItem(at: url) }
        let c = WorkbenchController(); _ = c.view
        try c.workspace.open(url); c.refreshDocument(); c.enterMode(.annotation)
        let window = NSWindow(contentRect: c.view.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = c; c.view.layoutSubtreeIfNeeded(); c.pdfView.layoutDocumentView()
        return (c, window)
    }
    private func button(_ id: String, _ c: WorkbenchController) throws -> ToolStripButton {
        try XCTUnwrap(descendants(c.view).compactMap { $0 as? ToolStripButton }.first { $0.toolID == id })
    }
    private func isArmed(_ c: WorkbenchController) -> Bool {
        let armed = c.pdfView.accessibilityLabel()?.contains("拖选") == true
        XCTAssertTrue(armed, "A markup button must enter text-drag mode instead of requiring an existing selection")
        return armed
    }
    private func activate(_ button: NSButton) {
        // Prevent a regression to a modal-only action from hanging the test runner.
        let dismiss = Timer(timeInterval: 0.02, repeats: true) { _ in
            if let modal = NSApp.modalWindow { NSApp.stopModal(); modal.orderOut(nil) }
        }
        RunLoop.main.add(dismiss, forMode: .modalPanel)
        RunLoop.main.add(dismiss, forMode: .common)
        button.performClick(nil); dismiss.invalidate()
    }
    private func drag(_ c: WorkbenchController, from first: Int, to last: Int) throws {
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        let a = page.characterBounds(at: first), b = page.characterBounds(at: last)
        send(.leftMouseDown, CGPoint(x: a.midX, y: a.midY), page, c)
        send(.leftMouseDragged, CGPoint(x: b.midX, y: b.midY), page, c)
        send(.leftMouseUp, CGPoint(x: b.midX, y: b.midY), page, c)
    }
    private func send(_ type: NSEvent.EventType, _ point: CGPoint, _ page: PDFPage, _ c: WorkbenchController) {
        let location = c.pdfView.convert(c.pdfView.convert(point, from: page), to: nil)
        let event = NSEvent.mouseEvent(with: type, location: location, modifierFlags: [], timestamp: 0,
            windowNumber: c.pdfView.window?.windowNumber ?? 0, context: nil, eventNumber: 1, clickCount: 1, pressure: 1)!
        switch type {
        case .leftMouseDown: c.pdfView.mouseDown(with: event)
        case .leftMouseDragged: c.pdfView.mouseDragged(with: event)
        default: c.pdfView.mouseUp(with: event)
        }
    }
    private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
}
