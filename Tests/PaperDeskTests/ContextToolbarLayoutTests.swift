import AppKit
import XCTest
@testable import PaperDesk

@MainActor final class ContextToolbarLayoutTests: XCTestCase {
    func testSelectedShapeHasNoEmptyToolRowAndDeselectCollapsesPalette() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank()
        try c.workspace.addShape(.rectangle, page: 0, bounds: CGRect(x: 20, y: 40, width: 80, height: 60), style: .init(strokeColor: .red, fillColor: nil, lineWidth: 2))
        c.refreshDocument()
        let w = NSWindow(contentRect: CGRect(x: 0, y: 0, width: 1280, height: 820), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        w.contentViewController = c; defer { w.contentViewController = nil }
        for size in [CGSize(width: 1280, height: 820), CGSize(width: 1180, height: 720), CGSize(width: 1600, height: 1000)] {
            w.setContentSize(size)
            c.pdfView.onAnnotationSelection?(0, 0); c.view.layoutSubtreeIfNeeded()
            let style = c.shapeStyleBar.convert(c.shapeStyleBar.bounds, to: c.view)
            XCTAssertLessThanOrEqual(c.view.bounds.maxY - c.workbenchToolbarHeight - style.maxY, 6, "No reserved blank tool row")
            let content = c.pdfView.convert(c.pdfView.bounds, to: c.view)
            XCTAssertEqual(c.view.bounds.maxY - content.maxY - c.workbenchToolbarHeight, 50, accuracy: 0.5)
            c.resetDocumentInteractionState(); c.view.layoutSubtreeIfNeeded()
            let collapsed = c.pdfView.convert(c.pdfView.bounds, to: c.view)
            XCTAssertEqual(c.view.bounds.maxY - collapsed.maxY, c.workbenchToolbarHeight, accuracy: 0.5)
        }
        c.enterMode(.annotation); c.pdfView.onAnnotationSelection?(0, 0); c.view.layoutSubtreeIfNeeded()
        let content = c.pdfView.convert(c.pdfView.bounds, to: c.view)
        XCTAssertEqual(c.view.bounds.maxY - content.maxY - c.workbenchToolbarHeight, 84, accuracy: 0.5, "A real tool row must remain visible")
    }
}
