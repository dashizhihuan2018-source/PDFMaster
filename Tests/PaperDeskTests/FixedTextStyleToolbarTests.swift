import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor final class FixedTextStyleToolbarTests: XCTestCase {
    private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    private func fixture() throws -> (WorkbenchController, NSWindow, PDFInlineTextSession, NSView) {
        _ = NSApplication.shared
        let controller = WorkbenchController(); _ = controller.view
        let window = NSWindow(contentRect: CGRect(x: 0, y: 0, width: 1100, height: 800), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.contentViewController = controller
        try controller.workspace.createBlank(); controller.refreshDocument()
        controller.beginFreeTextInput(from: CGPoint(x: 100, y: 650), to: CGPoint(x: 100, y: 650), page: 0)
        let session = try XCTUnwrap(controller.pdfView.inlineTextSession)
        session.textView.insertText("Fixed toolbar draft", replacementRange: NSRange(location: NSNotFound, length: 0))
        controller.view.layoutSubtreeIfNeeded()
        let family = try XCTUnwrap(descendants(controller.view).compactMap { $0 as? NSComboBox }.first { $0.accessibilityLabel() == "文字字体" })
        return (controller, window, session, try XCTUnwrap(family.superview))
    }

    func testStylesLiveOutsidePDFPageInFixedTopToolbar() throws {
        let (controller, window, session, toolbar) = try fixture(); defer { _ = session.finish(commit: false); window.contentViewController = nil }
        XCTAssertFalse(toolbar.isDescendant(of: controller.pdfView), "Style controls belong to the fixed workbench toolbar, not the page overlay")
        let top = toolbar.convert(toolbar.bounds, to: controller.view)
        let page = controller.pdfView.convert(controller.pdfView.bounds, to: controller.view)
        XCTAssertGreaterThanOrEqual(top.minY, page.maxY, "The style row must be above the PDF viewport")
    }

    func testZoomScrollAndRepositionNeverMoveTopStyleToolbar() throws {
        let (controller, window, session, toolbar) = try fixture(); defer { _ = session.finish(commit: false); window.contentViewController = nil }
        let frame = toolbar.convert(toolbar.bounds, to: controller.view)
        controller.pdfView.scaleFactor = 1.8; controller.pdfView.layoutDocumentView()
        controller.pdfView.documentView?.scroll(CGPoint(x: 0, y: 300))
        session.reposition(); controller.view.layoutSubtreeIfNeeded()
        let after = toolbar.convert(toolbar.bounds, to: controller.view)
        XCTAssertEqual(after.minX, frame.minX, accuracy: 0.01)
        XCTAssertEqual(after.minY, frame.minY, accuracy: 0.01)
        XCTAssertEqual(after.size.width, frame.size.width, accuracy: 0.01)
        XCTAssertEqual(after.size.height, frame.size.height, accuracy: 0.01)
    }

    func testClickTopToolbarPreservesDraftAndFinishHidesStyleHost() throws {
        let (controller, window, session, toolbar) = try fixture(); defer { _ = controller.pdfView.finishInlineTextEditing(commit: false); window.contentViewController = nil }
        let host = try XCTUnwrap(toolbar.superview)
        XCTAssertFalse(host.isHidden)
        // The top-right inset of the host is deliberately outside the editable
        // page, but inside the style surface. Clicking it must not commit.
        let point = host.convert(CGPoint(x: max(1, host.bounds.maxX - 2), y: host.bounds.midY), to: nil)
        let event = try XCTUnwrap(NSEvent.mouseEvent(with: .leftMouseDown, location: point, modifierFlags: [], timestamp: 1, windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 0))
        NSApp.sendEvent(event)
        XCTAssertTrue(controller.pdfView.inlineTextSession === session)
        XCTAssertFalse(controller.workspace.isDirty)
        XCTAssertTrue(session.finish())
        XCTAssertTrue(host.isHidden)
        XCTAssertNil(controller.pdfView.inlineTextSession)
        controller.beginFreeTextInput(from: CGPoint(x: 100, y: 500), to: CGPoint(x: 100, y: 500), page: 0)
        let next = try XCTUnwrap(controller.pdfView.inlineTextSession)
        XCTAssertFalse(host.isHidden)
        XCTAssertTrue(next.finish(commit: false))
        XCTAssertTrue(host.isHidden)
    }

    func testStandalonePDFViewHasNoFloatingStyleFallbackAndCanCommit() throws {
        _ = NSApplication.shared
        let workspace = PDFWorkspace(); try workspace.createBlank()
        let pdf = InkPDFView(frame: CGRect(x: 0, y: 0, width: 600, height: 800))
        pdf.document = try XCTUnwrap(PDFDocument(data: try XCTUnwrap(workspace.document?.dataRepresentation())))
        let page = try XCTUnwrap(pdf.document?.page(at: 0))
        var saved = ""
        let session = PDFInlineTextSession(pdfView: pdf, page: page,
            bounds: CGRect(x: 50, y: 500, width: 250, height: 80), text: "Standalone draft",
            font: .systemFont(ofSize: 14), color: .black,
            commit: { text, _, _, _ in saved = text }, failure: { XCTFail($0.localizedDescription) })
        pdf.inlineTextSession = session
        XCTAssertFalse(descendants(pdf).contains { $0 is NSComboBox }, "No fixed toolbar host means no style controls over the PDF")
        XCTAssertTrue(session.finish())
        XCTAssertEqual(saved, "Standalone draft")
    }
}
