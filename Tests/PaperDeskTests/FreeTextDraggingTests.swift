import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor final class FreeTextDraggingTests: XCTestCase {
    private func fixture(rotation: Int = 0, contentMode: Bool = true) throws -> (WorkbenchController, NSWindow) {
        _ = NSApplication.shared
        let c = WorkbenchController(); _ = c.view
        let w = NSWindow(contentRect: CGRect(x: 0, y: 0, width: 1280, height: 900), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        w.contentViewController = c
        try c.workspace.createBlank()
        if rotation != 0 { try c.workspace.rotate(page: 0, degrees: rotation) }
        try c.workspace.addText("拖动文字 Move me", page: 0, bounds: CGRect(x: 160, y: 360, width: 230, height: 90), font: NSFont(name: "Times-Bold", size: 24)!, color: .red)
        c.refreshDocument(); c.enterMode(.editing)
        if contentMode { c.selectContentTool("content-text") } else { c.addFreeText() }
        c.view.layoutSubtreeIfNeeded(); c.pdfView.scaleFactor = 0.8; c.pdfView.layoutDocumentView()
        c.pdfView.refreshCommittedInkRendering()
        return (c, w)
    }
    private func event(_ kind: NSEvent.EventType, at p: CGPoint, in view: InkPDFView, clicks: Int = 1) -> NSEvent {
        NSEvent.mouseEvent(with: kind, location: view.convert(p, to: nil), modifierFlags: [], timestamp: 1, windowNumber: view.window?.windowNumber ?? 0, context: nil, eventNumber: 0, clickCount: clicks, pressure: 1)!
    }
    private func point(_ c: WorkbenchController) throws -> CGPoint {
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        let b = try XCTUnwrap(page.annotations.first?.bounds)
        return c.pdfView.convert(CGPoint(x: b.midX, y: b.midY), from: page)
    }
    private func textLayers(_ layer: CALayer) -> [FreeTextAppearanceLayer] {
        (layer as? FreeTextAppearanceLayer).map { [$0] } ?? (layer.sublayers ?? []).flatMap(textLayers)
    }
    func testSingleClickSelectsWithoutTypingOrUndoAndDoubleClickEdits() throws {
        let (c, w) = try fixture(); defer { _ = c.pdfView.finishInlineTextEditing(commit: false); w.contentViewController = nil }
        let p = try point(c), revision = c.workspace.revision
        c.pdfView.mouseDown(with: event(.leftMouseDown, at: p, in: c.pdfView))
        c.pdfView.mouseUp(with: event(.leftMouseUp, at: p, in: c.pdfView))
        XCTAssertNil(c.pdfView.inlineTextSession, "A single click selects the object rather than capturing drags inside NSTextView")
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertNotNil(c.pdfView.renderShapeOverlay())
        c.pdfView.mouseDown(with: event(.leftMouseDown, at: p, in: c.pdfView, clicks: 2))
        XCTAssertNotNil(c.pdfView.inlineTextSession)
    }
    func testMoveInBothTextToolsAcrossAllRotationsPreservesStyleAndIsOneUndo() throws {
        for contentMode in [true, false] { for rotation in [0, 90, 180, 270] {
            let (c, w) = try fixture(rotation: rotation, contentMode: contentMode)
            defer { _ = c.pdfView.finishInlineTextEditing(commit: false); w.contentViewController = nil }
            let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
            let original = try XCTUnwrap(c.workspace.document?.page(at: 0)?.annotations.first?.bounds)
            let revision = c.workspace.revision
            let p = try point(c)
            c.pdfView.mouseDown(with: event(.leftMouseDown, at: p, in: c.pdfView))
            let selectedPixels = c.pdfView.renderShapeOverlay()
            let overlay = try XCTUnwrap(c.pdfView.subviews.first { $0.layer?.zPosition == 1000 }?.layer)
            let textLayer = try XCTUnwrap(textLayers(overlay).first)
            let initialFrame = textLayer.frame
            let start = c.pdfView.convert(p, to: page)
            let end = c.pdfView.convert(CGPoint(x: start.x + 45, y: start.y - 30), from: page)
            c.pdfView.mouseDragged(with: event(.leftMouseDragged, at: end, in: c.pdfView))
            XCTAssertEqual(c.workspace.revision, revision, "Dragging is only a live presentation preview")
            XCTAssertNotEqual(selectedPixels, c.pdfView.renderShapeOverlay(), "The preview must visibly move before release")
            XCTAssertEqual(textLayers(overlay).count, 1, "No duplicate ghost at the original position")
            XCTAssertEqual(textLayer.frame.minX - initialFrame.minX, end.x - p.x, accuracy: 1.1)
            XCTAssertEqual(textLayer.frame.minY - initialFrame.minY, end.y - p.y, accuracy: 1.1)
            c.pdfView.mouseUp(with: event(.leftMouseUp, at: end, in: c.pdfView))
            XCTAssertNil(c.pdfView.inlineTextSession)
            let moved = try XCTUnwrap(c.workspace.document?.page(at: 0)?.annotations.first)
            XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
            XCTAssertEqual(moved.bounds.minX, original.minX + 45, accuracy: 0.1)
            XCTAssertEqual(moved.bounds.minY, original.minY - 30, accuracy: 0.1)
            XCTAssertEqual(moved.bounds.size, original.size)
            XCTAssertEqual(moved.contents, "拖动文字 Move me")
            XCTAssertEqual(moved.font?.fontName, "Times-Bold")
            XCTAssertEqual(moved.font?.pointSize, 24)
            XCTAssertEqual(moved.fontColor?.usingColorSpace(.deviceRGB)?.redComponent ?? 0, 1, accuracy: 0.01)
            XCTAssertEqual(c.workspace.revision, revision + 1)
            let data = try XCTUnwrap(c.workspace.document?.dataRepresentation())
            let reopened = try XCTUnwrap(PDFDocument(data: data)?.page(at: 0)?.annotations.first?.bounds)
            XCTAssertEqual(reopened.minX, moved.bounds.minX, accuracy: 0.001)
            XCTAssertEqual(reopened.minY, moved.bounds.minY, accuracy: 0.001)
            c.undo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.bounds, original)
            c.redo()
            let redone = try XCTUnwrap(c.workspace.document?.page(at: 0)?.annotations.first?.bounds)
            XCTAssertEqual(redone.minX, moved.bounds.minX, accuracy: 0.001)
            XCTAssertEqual(redone.minY, moved.bounds.minY, accuracy: 0.001)
        } }
    }
    func testEscapeCancelsDragAndOutsidePageMovementClampsWholeTextBox() throws {
        let (c, w) = try fixture(); defer { w.contentViewController = nil }
        let original = try XCTUnwrap(c.workspace.document?.page(at: 0)?.annotations.first?.bounds)
        let revision = c.workspace.revision, p = try point(c)
        c.pdfView.mouseDown(with: event(.leftMouseDown, at: p, in: c.pdfView))
        c.pdfView.mouseDragged(with: event(.leftMouseDragged, at: CGPoint(x: p.x + 30, y: p.y + 20), in: c.pdfView))
        XCTAssertTrue(c.pdfView.cancelPlacementForEscape())
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.bounds, original)
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        c.pdfView.mouseDown(with: event(.leftMouseDown, at: p, in: c.pdfView))
        let far = c.pdfView.convert(CGPoint(x: 2000, y: -2000), from: page)
        c.pdfView.mouseDragged(with: event(.leftMouseDragged, at: far, in: c.pdfView))
        c.pdfView.mouseUp(with: event(.leftMouseUp, at: far, in: c.pdfView))
        let result = try XCTUnwrap(c.workspace.document?.page(at: 0)?.annotations.first?.bounds)
        XCTAssertTrue(page.bounds(for: .cropBox).contains(result))
        XCTAssertEqual(result.size, original.size)
    }
}
