import AppKit
import XCTest
@testable import PaperDesk

@MainActor final class ProductCopyTests: XCTestCase {
    func testSupportPageShowsProvidedEmailAndPrivacyReminder() {
        let recentURL = FileManager.default.temporaryDirectory.appendingPathComponent("support-test-\(UUID()).json")
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: recentURL))
        _ = shell.view
        shell.dashboardController.select(.support)
        func strings(_ view: NSView) -> [String] {
            let own = (view as? NSTextField).map { [$0.stringValue] } ?? []
            return own + view.subviews.flatMap(strings)
        }
        let text = strings(shell.dashboardController.view).joined(separator: "\n")
        XCTAssertTrue(text.contains("邮箱：archseuyang@163.com"))
        XCTAssertEqual(text.components(separatedBy: "archseuyang@163.com").count - 1, 1)
        XCTAssertFalse(text.contains("微信"))
        XCTAssertTrue(text.contains("请勿发送密码、API Key 或敏感文档"))
    }

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
