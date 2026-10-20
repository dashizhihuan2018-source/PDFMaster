import AppKit
import PaperCore
import WebKit
import XCTest
@testable import PaperDesk

@MainActor final class WubianjiEntryTests: XCTestCase {
    private func shell() throws -> ApplicationShellController {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("wubianji-entry-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: directory.appendingPathComponent("recent.json")))
        shell.view.frame = CGRect(x: 0, y: 0, width: 1280, height: 820)
        return shell
    }
    private func descendants(_ view: NSView) -> [NSView] {
        [view] + view.subviews.flatMap(descendants)
    }
    private func buttons(_ view: NSView) -> [NSButton] { descendants(view).compactMap { $0 as? NSButton } }
    private func stopWebViews(_ view: NSView) { descendants(view).compactMap { $0 as? WKWebView }.forEach { $0.stopLoading() } }

    func testPrimaryCanvasNavigationIsLocalWubianjiWithoutWebsite() throws {
        let shell = try shell(), dashboard = shell.dashboardController
        let navigation = try XCTUnwrap(buttons(dashboard.view).first { $0.title == "无边记" })
        navigation.performClick(nil)
        defer { stopWebViews(dashboard.view) }
        XCTAssertEqual(dashboard.selectedSection, .canvas)
        XCTAssertEqual(HomeSection.canvas.rawValue, "无边记")
        XCTAssertNotNil(buttons(dashboard.view).first { $0.title == "无边记" })
        let online = try XCTUnwrap(buttons(dashboard.view).first { $0.title == "无限画布" })
        XCTAssertTrue(online.toolTip?.contains("内置") == true)
        XCTAssertFalse(descendants(dashboard.view).contains { $0 is WKWebView }, "本地无边记不能由在线网站顶替")
        XCTAssertEqual(shell.canvasTabs.count, 0, "进入画布库不应自动创建空白存档")
    }

    func testVisibleNewButtonCreatesNativeBoardAndBrandedTab() throws {
        let shell = try shell(), dashboard = shell.dashboardController
        dashboard.select(.canvas)
        defer { stopWebViews(dashboard.view) }
        shell.view.layoutSubtreeIfNeeded()
        let create = try XCTUnwrap(buttons(dashboard.view).first { $0.title == "新建无边记" })
        XCTAssertGreaterThanOrEqual(create.frame.height, 32)
        XCTAssertGreaterThanOrEqual(create.frame.width, 110)
        create.performClick(nil)
        let canvas = try XCTUnwrap(shell.activeCanvas)
        XCTAssertEqual(shell.canvasTabs.count, 1)
        XCTAssertEqual(canvas.session.board.title, "未命名无边记")
        XCTAssertNil(canvas.sourceURL)
        XCTAssertNil(shell.activeWorkbench)
        XCTAssertTrue(buttons(shell.view).contains { $0.title.contains("无边记") && $0.accessibilityLabel()?.hasPrefix("无边记标签 ") == true })
        XCTAssertFalse(descendants(dashboard.view).contains { $0 is WKWebView })
    }

    func testVisibleExistingBoardButtonReactivatesSameUnsavedBoard() throws {
        let shell = try shell()
        let existing = shell.createCanvas(board: CanvasBoard(title: "课程灵感"))
        try existing.controller.session.mutate { $0.elements.append(CanvasElement(kind: .sticky, frame: CGRect(x: 0, y: 0, width: 120, height: 100), text: "保留内容")) }
        shell.showHome()
        let dashboard = shell.dashboardController
        dashboard.select(.canvas)
        defer { stopWebViews(dashboard.view) }
        let entry = try XCTUnwrap(buttons(dashboard.view).first { $0.title.contains("课程灵感") })
        entry.performClick(nil)
        XCTAssertTrue(shell.activeCanvas === existing.controller)
        XCTAssertEqual(shell.canvasTabs.count, 1)
        XCTAssertEqual(existing.controller.session.board.elements.first?.text, "保留内容")
        XCTAssertTrue(existing.controller.session.isDirty)
    }

    func testOpenEntryKeepsExistingPDFMasterboardFormatAndStoredTitle() throws {
        let shell = try shell(), dashboard = shell.dashboardController
        dashboard.select(.canvas)
        defer { stopWebViews(dashboard.view) }
        let open = try XCTUnwrap(buttons(dashboard.view).first { $0.title == "打开无边记…" })
        XCTAssertTrue(open.target === dashboard)
        XCTAssertEqual(open.action, NSSelectorFromString("openCanvas"))
        let url = shell.recentStore.url.deletingLastPathComponent().appendingPathComponent("legacy.pdfmasterboard")
        let old = CanvasBoard(title: "原来的无限画布", elements: [CanvasElement(kind: .text, frame: CGRect(x: 1, y: 2, width: 120, height: 40), text: "旧存档")])
        try old.save(to: url)
        let original = try Data(contentsOf: url)
        XCTAssertTrue(shell.open(url: url))
        XCTAssertEqual(shell.activeCanvas?.session.board, old)
        XCTAssertEqual(shell.activeCanvas?.sourceURL, url)
        XCTAssertEqual(try Data(contentsOf: url), original, "重命名入口不迁移、不重写旧存档")
        XCTAssertTrue(shell.recentStore.recents.isEmpty)
    }

    func testOnlineCanvasHasSeparateRouteAndDoesNotReplaceNativeBoard() throws {
        let shell = try shell()
        let native = shell.createCanvas(board: CanvasBoard(title: "本地草稿"))
        shell.showHome()
        let dashboard = shell.dashboardController
        let online = try XCTUnwrap(HomeSection(rawValue: "无限画布"))
        dashboard.select(online)
        defer { stopWebViews(dashboard.view) }
        let web = try XCTUnwrap(descendants(dashboard.view).compactMap { $0 as? WKWebView }.first)
        XCTAssertEqual(web.url?.scheme, "pdfmaster-canvas")
        web.stopLoading()
        dashboard.select(.canvas)
        XCTAssertTrue(web.isHidden || web.superview?.isHidden == true)
        XCTAssertNotNil(buttons(dashboard.view).first { $0.title == "新建无边记" })
        XCTAssertTrue(shell.canvasTabs.first === native)
        XCTAssertEqual(shell.canvasTabs.count, 1)
    }
}
