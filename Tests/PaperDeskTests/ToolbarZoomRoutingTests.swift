import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor
final class ToolbarZoomRoutingTests: XCTestCase {
    func testActualFitButtonUsesFitPageInsteadOfContinuousAutoFitWidth() async throws {
        let controller = try fixture()
        controller.readerSession.setLayout(.single, continuous: true)
        try button("适合页面", in: controller.view).performClick(nil)
        await settle(controller.pdfView)
        XCTAssertEqual(controller.readerSession.currentZoom, .fitPage)
        XCTAssertFalse(controller.pdfView.autoScales)
        let page = try XCTUnwrap(controller.pdfView.currentPage)
        let rect = controller.pdfView.convert(page.bounds(for: .cropBox), from: page)
        XCTAssertLessThanOrEqual(rect.height, controller.pdfView.bounds.height - 24)
        XCTAssertLessThanOrEqual(rect.width, controller.pdfView.bounds.width - 24)
    }

    func testActualPlusAndMinusButtonsCancelFitAndKeepPercentOnResize() async throws {
        for (title, multiplier) in [("放大", 1.25), ("缩小", 0.8)] {
            let controller = try fixture()
            try controller.readerSession.setZoom(.fitPage)
            let original = controller.pdfView.scaleFactor
            try button(title, in: controller.view).performClick(nil)
            let changed = controller.pdfView.scaleFactor
            XCTAssertEqual(changed, original * multiplier, accuracy: 1e-10, title)
            if case .percent(let requested) = controller.readerSession.currentZoom {
                XCTAssertEqual(requested, original * multiplier * 100, accuracy: 1e-10, title)
            } else { XCTFail("\(title) must cancel fit before changing the scale") }
            controller.pdfView.frame.size = CGSize(width: controller.pdfView.frame.width * 0.6,
                                                   height: controller.pdfView.frame.height * 0.7)
            await settle(controller.pdfView)
            XCTAssertEqual(controller.pdfView.scaleFactor, changed, accuracy: 1e-10, title)
            guard case .percent = controller.readerSession.currentZoom else {
                XCTFail("\(title) must leave the reader in fixed percentage mode")
                continue
            }
        }
    }

    func testActualZoomButtonsPreservePageOverviewInterception() async throws {
        let controller = try fixture()
        try controller.readerSession.setZoom(.percent(125))
        try button("页面预览", in: controller.view).performClick(nil)
        XCTAssertTrue(controller.isOverviewVisible)
        let initial = controller.pageOverview.thumbnailScale
        try button("放大", in: controller.view).performClick(nil)
        XCTAssertEqual(controller.pageOverview.thumbnailScale, initial * 1.2, accuracy: 1e-10)
        try button("缩小", in: controller.view).performClick(nil)
        XCTAssertEqual(controller.pageOverview.thumbnailScale, initial, accuracy: 1e-10)
        controller.pageOverview.setThumbnailScale(1.6)
        try button("适合页面", in: controller.view).performClick(nil)
        XCTAssertEqual(controller.pageOverview.thumbnailScale, 1)
        await settle(controller.pdfView)
        XCTAssertTrue(controller.isOverviewVisible)
        XCTAssertEqual(controller.readerSession.currentZoom, .percent(125))
        XCTAssertEqual(controller.pdfView.scaleFactor, 1.25, accuracy: 1e-10)
    }

    private func fixture() throws -> WorkbenchController {
        let controller = WorkbenchController()
        controller.view.frame = NSRect(x: 0, y: 0, width: 1200, height: 580)
        try controller.workspace.createBlank()
        controller.refreshDocument()
        controller.view.layoutSubtreeIfNeeded()
        controller.pdfView.layoutDocumentView()
        XCTAssertGreaterThan(controller.pdfView.bounds.width, 500)
        XCTAssertGreaterThan(controller.pdfView.bounds.height, 200)
        return controller
    }

    private func button(_ label: String, in view: NSView) throws -> NSButton {
        if let button = view as? NSButton, button.accessibilityLabel() == label { return button }
        for child in view.subviews {
            if let found = try? button(label, in: child) { return found }
        }
        throw NSError(domain: "ToolbarZoomRoutingTests", code: 1, userInfo: [NSLocalizedDescriptionKey: "Missing toolbar button: \(label)"])
    }

    private func settle(_ view: PDFView) async {
        view.layoutSubtreeIfNeeded()
        view.layoutDocumentView()
        for _ in 0..<12 { await Task.yield() }
    }
}
