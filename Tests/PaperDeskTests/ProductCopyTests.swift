import AppKit
import XCTest
@testable import PaperDesk

@MainActor final class ProductCopyTests: XCTestCase {
    func testReaderStatusUsesProductCopy() {
        XCTAssertEqual(DocumentStatusPresentation.message(hasDocument: true, isEncryptedSource: false), "PDF Master · 本地阅读与批注")
        XCTAssertEqual(DocumentStatusPresentation.message(hasDocument: false, isEncryptedSource: false), "PDF Master · 本地阅读与批注")
    }

    func testUnavailableToolsStateSupportBoundaryWithoutDevelopmentLanguage() {
        for tool in ToolCatalog.all where tool.availability == .unavailable {
            XCTAssertEqual(tool.unavailableReason, "暂不支持此功能")
        }
    }

    func testInitialWorkbenchDoesNotShowPrototypeDisclaimer() {
        let controller = WorkbenchController(); _ = controller.view
        func strings(_ view: NSView) -> [String] {
            (view as? NSTextField).map { [$0.stringValue] } ?? [] + view.subviews.flatMap(strings)
        }
        let text = strings(controller.view).joined(separator: "\n")
        XCTAssertFalse(text.contains("开发版本"))
        XCTAssertFalse(text.contains("持续完善"))
    }
}
