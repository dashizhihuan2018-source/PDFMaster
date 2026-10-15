import AppKit
import WebKit
import XCTest
@testable import PaperDesk

final class CanvasWebsiteTests: XCTestCase {
    @MainActor private func shell() -> ApplicationShellController {
        ApplicationShellController(recentStore: RecentDocumentStore(url: FileManager.default.temporaryDirectory.appendingPathComponent("website-test-\(UUID()).json")))
    }
    @MainActor private func webViews(in view: NSView) -> [WKWebView] {
        (view as? WKWebView).map { [$0] } ?? view.subviews.flatMap { webViews(in: $0) }
    }

    @MainActor func testCanvasSectionHostsWholeWebsiteEdgeToEdgeInsteadOfAnEntryButton() throws {
        let shell = shell()
        shell.view.frame = NSRect(x: 0, y: 0, width: 1280, height: 820)
        let dashboard = shell.dashboardController
        dashboard.select(.canvas)
        shell.view.layoutSubtreeIfNeeded()
        let web = try XCTUnwrap(webViews(in: dashboard.view).first, "无限画布必须直接展示整个网站，不能只有入口按钮")
        XCTAssertEqual(web.url?.host, "infinitecanvas.maip.top")
        XCTAssertEqual(web.url?.path, "/canvas")
        XCTAssertEqual(URLComponents(url: try XCTUnwrap(web.url), resolvingAgainstBaseURL: false)?.queryItems?.first(where: { $0.name == "mode" })?.value, "recent", "进入时直接恢复原网站画布，不停在只有新建按钮的画布库")
        XCTAssertGreaterThanOrEqual(web.frame.width, 1050)
        XCTAssertGreaterThan(web.frame.height, 700)
        XCTAssertTrue(web.configuration.websiteDataStore.isPersistent)
        XCTAssertTrue(web.configuration.userContentController.userScripts.isEmpty, "不注入 PDF、API Key 或替代网站 UI")
        web.stopLoading()
    }

    @MainActor func testSwitchingHomeAndPDFTabsKeepsSameWebsiteAndNativeBoards() throws {
        let shell = shell()
        _ = shell.view
        shell.dashboardController.select(.canvas)
        let original = try XCTUnwrap(webViews(in: shell.dashboardController.view).first)
        original.stopLoading()
        let native = shell.createCanvas()
        shell.showHome()
        shell.dashboardController.select(.home)
        shell.dashboardController.select(.canvas)
        let restored = try XCTUnwrap(webViews(in: shell.dashboardController.view).first)
        XCTAssertTrue(restored === original, "切换页面不能重建网站或丢失生成面板状态")
        XCTAssertEqual(shell.canvasTabs.count, 1)
        XCTAssertTrue(shell.canvasTabs.first === native)
    }

    @MainActor func testFileSaverPopupIsRetainedSoExportCanReachWebKitDownloadDelegate() throws {
        let controller = CanvasWebsiteController(); _ = controller.view
        controller.webView.stopLoading()
        let popup = controller.webView(controller.webView, createWebViewWith: WKWebViewConfiguration(), for: WKNavigationAction(), windowFeatures: WKWindowFeatures())
        XCTAssertNotNil(popup, "原站 FileSaver 在 macOS WebView 先打开空白窗口，再填入导出文件；返回 nil 会静默丢失下载")
        XCTAssertTrue(popup?.navigationDelegate === controller)
    }

    @MainActor func testEmbeddedNavigationTrustsOnlyExactWebsiteHTTPSOrigin() {
        XCTAssertTrue(CanvasWebsiteController.isWebsiteURL(URL(string: "https://infinitecanvas.maip.top/login")!))
        for address in ["http://infinitecanvas.maip.top/", "https://infinitecanvas.maip.top.evil.example/", "https://infinitecanvas.maip.top:8443/", "https://user:password@infinitecanvas.maip.top/", "file:///tmp/private.pdf", "javascript:alert(1)"] {
            XCTAssertFalse(CanvasWebsiteController.isWebsiteURL(URL(string: address)!), address)
        }
    }
}
