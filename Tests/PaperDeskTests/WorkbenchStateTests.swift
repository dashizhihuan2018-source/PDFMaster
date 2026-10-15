import XCTest
import PDFKit
@testable import PaperDesk

final class WorkbenchStateTests: XCTestCase {
    func testSearchSummaryDistinguishesEmptyAndNoMatch() {
        XCTAssertEqual(SearchSummary(query: "", resultCount: 0, currentIndex: nil).text, "输入关键词搜索")
        XCTAssertEqual(SearchSummary(query: "梁", resultCount: 0, currentIndex: nil).text, "未找到“梁”")
        XCTAssertEqual(SearchSummary(query: "梁", resultCount: 3, currentIndex: 1).text, "2 / 3")
    }

    func testPageJumpAcceptsOnlyOneBasedExistingPages() {
        XCTAssertEqual(PageJump.index(for: "1", pageCount: 4), 0)
        XCTAssertEqual(PageJump.index(for: " 4 ", pageCount: 4), 3)
        XCTAssertNil(PageJump.index(for: "0", pageCount: 4))
        XCTAssertNil(PageJump.index(for: "5", pageCount: 4))
        XCTAssertNil(PageJump.index(for: "x", pageCount: 4))
    }

    func testChineseErrorMessagesRemainActionable() {
        XCTAssertTrue(WorkbenchMessage.error(.cannotOverwriteSource).contains("原文件"))
        XCTAssertTrue(WorkbenchMessage.error(.permissionDenied).contains("权限"))
        XCTAssertTrue(WorkbenchMessage.error(.cannotDeleteLastPage).contains("最后一页"))
    }

    func testInteractiveWidgetAdmissionRejectsForms() throws {
        let document = PDFDocument()
        let image = NSImage(size: NSSize(width: 100, height: 100))
        image.lockFocus(); NSColor.white.setFill(); NSRect(x: 0, y: 0, width: 100, height: 100).fill(); image.unlockFocus()
        let page = try XCTUnwrap(PDFPage(image: image))
        document.insert(page, at: 0)
        XCTAssertFalse(DocumentAdmission.hasInteractiveWidgets(document))
        page.addAnnotation(PDFAnnotation(bounds: CGRect(x: 10, y: 10, width: 20, height: 20), forType: .widget, withProperties: nil))
        XCTAssertTrue(DocumentAdmission.hasInteractiveWidgets(document))
    }

    func testSignaturePlacementKeepsStrokesSeparateAndRotatesAroundCropCenter() {
        let strokes = [[CGPoint(x: 0, y: 0.5), CGPoint(x: 1, y: 0.5)],
                       [CGPoint(x: 0.5, y: 0), CGPoint(x: 0.5, y: 1)]]
        let box = CGRect(x: 100, y: 200, width: 400, height: 600)
        let zero = SignaturePlacement.map(strokes: strokes, cropBox: box, rotation: 0)
        XCTAssertEqual(zero.count, 2)
        XCTAssertEqual(zero[0][0].x, 220, accuracy: 0.01)
        XCTAssertEqual(zero[0][1].x, 380, accuracy: 0.01)
        let ninety = SignaturePlacement.map(strokes: strokes, cropBox: box, rotation: 90)
        XCTAssertEqual(ninety[0][0].y, 420, accuracy: 0.01)
        XCTAssertEqual(ninety[0][1].y, 580, accuracy: 0.01)
    }

    @MainActor func testWorkbenchInitialViewHasUsableDesktopFrame() {
        let controller = WorkbenchController()
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820),
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.contentViewController = controller
        window.contentMinSize = NSSize(width: 1180, height: 700)
        window.setContentSize(NSSize(width: 1280, height: 820))
        controller.view.layoutSubtreeIfNeeded()

        XCTAssertGreaterThanOrEqual(window.contentLayoutRect.width, 1180)
        XCTAssertGreaterThanOrEqual(window.contentLayoutRect.height, 700)
        XCTAssertGreaterThan(controller.pdfView.frame.width, 1000)
        XCTAssertGreaterThan(controller.pdfView.frame.height, 600)
    }
}
