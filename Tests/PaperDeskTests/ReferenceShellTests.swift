import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

final class ReferenceShellTests: XCTestCase {
    func testCatalogMatchesReferenceExactly() {
        XCTAssertEqual(ToolCatalog.all.count, 28)
        XCTAssertEqual(Set(ToolCatalog.all.map(\.id)).count, 28)
        XCTAssertEqual(ToolGroup.allCases.map { group in ToolCatalog.all.filter { $0.group == group }.count }, [6, 6, 8, 4, 4])
        XCTAssertEqual(ToolCatalog.all.map(\.title), [
            "批注", "编辑", "签署 PDF", "填写表单", "评论 PDF", "页面重设大小",
            "识别文本", "管理页面", "压缩 PDF", "合并 PDF", "对比 PDF", "页码",
            "PDF Copilot", "空白 PDF", "度量", "设置密码", "翻译 PDF", "隐藏 PDF 内容", "红线", "绘图",
            "图片转 PDF", "Word 转 PDF", "表转 PDF", "其他文件转 PDF",
            "PDF 转 Word", "PDF 转图片", "PDF 转 Excel", "PDF 转 PowerPoint"
        ])
    }

    func testRecommendedSelectionValidatesCapacityAndPersists() throws {
        let url = temporaryDirectory().appendingPathComponent("preferences.json")
        let preferences = PreferredToolStore(url: url)
        XCTAssertEqual(preferences.toolIDs.count, 6)
        XCTAssertThrowsError(try preferences.replace(with: ["annotate"]))
        XCTAssertThrowsError(try preferences.replace(with: Array(repeating: "annotate", count: 6)))
        let chosen = ["blank", "annotate", "pages", "measure", "sign", "copilot"]
        try preferences.replace(with: chosen)
        XCTAssertEqual(PreferredToolStore(url: url).toolIDs, chosen)
    }

    func testRecentStorePersistsDedupeFavoriteAndRemoval() throws {
        let directory = temporaryDirectory()
        let json = directory.appendingPathComponent("recent.json")
        let pdf = directory.appendingPathComponent("a.pdf")
        try Data("x".utf8).write(to: pdf)
        let store = RecentDocumentStore(url: json)
        try store.recordSuccessfulOpen(pdf)
        try store.recordSuccessfulOpen(pdf.appendingPathComponent("..", isDirectory: true).appendingPathComponent("a.pdf"))
        XCTAssertEqual(store.entries.count, 1)
        try store.toggleFavorite(pdf)
        XCTAssertTrue(RecentDocumentStore(url: json).entries[0].isFavorite)
        try store.removeFromRecents(pdf)
        XCTAssertTrue(store.entries[0].isFavorite, "favorites survive removal from recents")
    }

    func testRecentStoreSurfacesSaveErrorAndKeepsMissingFavoriteMetadata() throws {
        let directory = temporaryDirectory()
        let json = directory.appendingPathComponent("recent.json")
        let pdf = directory.appendingPathComponent("missing.pdf")
        try Data("x".utf8).write(to: pdf)
        let store = RecentDocumentStore(url: json)
        try store.recordSuccessfulOpen(pdf)
        try store.toggleFavorite(pdf)
        try FileManager.default.removeItem(at: pdf)
        XCTAssertFalse(store.entries[0].exists)
        XCTAssertThrowsError(try RecentDocumentStore(url: directory).recordSuccessfulOpen(URL(fileURLWithPath: "/tmp/nope.pdf")))
    }

    @MainActor func testInvalidAdmissionAndCloseCancellationLeaveTabsUntouched() throws {
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: temporaryDirectory().appendingPathComponent("r.json")))
        let invalid = temporaryDirectory().appendingPathComponent("bad.pdf")
        try Data("bad".utf8).write(to: invalid)
        XCTAssertFalse(shell.open(url: invalid, passwordPrompt: { nil }))
        XCTAssertEqual(shell.documentTabs.count, 0)
        let tab = shell.createBlankDocument()
        try tab.controller.workspace.addPopupNote("dirty", author: nil, page: 0, bounds: CGRect(x: 10, y: 10, width: 20, height: 20), color: .yellow)
        XCTAssertFalse(shell.close(tabID: tab.id, dirtyDecision: { false }))
        XCTAssertEqual(shell.documentTabs.count, 1)
    }

    @MainActor func testTabsOwnIndependentWorkspacesAndActiveRouting() throws {
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: temporaryDirectory().appendingPathComponent("r.json")))
        let first = shell.createBlankDocument()
        let second = shell.createBlankDocument()
        try first.controller.workspace.addPopupNote("one", author: nil, page: 0, bounds: CGRect(x: 10, y: 10, width: 20, height: 20), color: .yellow)
        XCTAssertNotEqual(first.controller.workspace.revision, second.controller.workspace.revision)
        shell.activate(tabID: first.id)
        XCTAssertTrue(shell.activeWorkbench === first.controller)
    }

    @MainActor func testWorkbenchDarkReaderGeometryAtSupportedSizes() {
        let controller = WorkbenchController(); _ = controller.view
        XCTAssertEqual(controller.pdfView.backgroundColor, ShellPalette.canvas)
        XCTAssertEqual(controller.workbenchToolbarHeight, 46)
        XCTAssertEqual(controller.thumbnailSidebarWidth, 176)
        for size in [NSSize(width: 1280, height: 820), NSSize(width: 1180, height: 700)] {
            controller.view.frame.size = size
            controller.view.layoutSubtreeIfNeeded()
            XCTAssertGreaterThan(controller.pdfView.frame.width, size.width - 240)
            XCTAssertGreaterThan(controller.pdfView.frame.height, size.height - 90)
        }
    }

    func testRecentStoreCapsNonFavoritesButKeepsFavoritesAndRollsBackSaveFailure() throws {
        let directory = temporaryDirectory()
        let json = directory.appendingPathComponent("recent.json")
        let store = RecentDocumentStore(url: json)
        for index in 0..<52 {
            let file = directory.appendingPathComponent("\(index).pdf")
            try Data().write(to: file)
            try store.recordSuccessfulOpen(file, at: Date(timeIntervalSince1970: TimeInterval(index)))
            if index == 0 { try store.toggleFavorite(file) }
        }
        XCTAssertEqual(store.recents.filter { !$0.isFavorite }.count, 50)
        XCTAssertEqual(store.favorites.count, 1)
        let reloaded = RecentDocumentStore(url: json)
        XCTAssertEqual(reloaded.recents.first?.normalizedURL.lastPathComponent, "51.pdf")

        let before = reloaded.entries
        try FileManager.default.removeItem(at: json)
        try FileManager.default.createDirectory(at: json, withIntermediateDirectories: true)
        XCTAssertThrowsError(try reloaded.clearRecents())
        XCTAssertEqual(reloaded.entries, before)
    }

    @MainActor func testToolDispatchTargetsMostRecentDocumentFromHomeWithoutMutatingForModes() throws {
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: temporaryDirectory().appendingPathComponent("r.json")))
        let first = shell.createBlankDocument()
        let second = shell.createBlankDocument()
        shell.activate(tabID: first.id)
        shell.showHome()
        let revision = first.controller.workspace.revision
        XCTAssertTrue(shell.dispatch(toolID: "annotate", openProvider: { nil }))
        XCTAssertTrue(shell.activeWorkbench === first.controller)
        XCTAssertEqual(first.controller.workspace.revision, revision)
        XCTAssertEqual(first.controller.activeMode, .annotation)
        XCTAssertNil(second.controller.activeMode)
    }

    @MainActor func testToolDispatchAfterAdmissionAndAllDirtyCancellation() throws {
        let directory = temporaryDirectory()
        let source = directory.appendingPathComponent("source.pdf")
        let document = PDFDocument(); document.insert(try XCTUnwrap(PDFPage(image: whiteImage())), at: 0)
        XCTAssertTrue(document.write(to: source))
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: directory.appendingPathComponent("r.json")))
        XCTAssertTrue(shell.dispatch(toolID: "pages", openProvider: { source }))
        XCTAssertEqual(shell.documentTabs.count, 1)
        XCTAssertEqual(shell.activeWorkbench?.activeMode, .pages)
        let blank = shell.createBlankDocument()
        try blank.controller.workspace.addPopupNote("dirty", author: nil, page: 0, bounds: CGRect(x: 2, y: 2, width: 10, height: 10), color: .yellow)
        XCTAssertFalse(shell.guardAllDirty { $0.id != blank.id })
        XCTAssertEqual(shell.documentTabs.count, 2)
    }

    @MainActor func testDashboardResponsiveGridSelectionAndDropAdmission() throws {
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: temporaryDirectory().appendingPathComponent("r.json")))
        let dashboard = DashboardViewController(shell: shell); _ = dashboard.view
        dashboard.view.frame = NSRect(x: 0, y: 0, width: 1280, height: 780)
        dashboard.select(.tools); dashboard.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(dashboard.renderedColumnCount, 6)
        dashboard.view.frame.size.width = 1040
        dashboard.view.layoutSubtreeIfNeeded()
        dashboard.refreshResponsiveLayout()
        XCTAssertLessThan(dashboard.renderedColumnCount, 6)
        XCTAssertEqual(dashboard.selectedSection, .tools)
        XCTAssertTrue(dashboard.acceptsLocalPDFDrop)
    }

    @MainActor func testWorkbenchToolbarPageBadgeAndSidebarToggleUseRealGeometry() throws {
        let controller = WorkbenchController(); _ = controller.view
        try controller.workspace.createBlank(); controller.refreshDocument()
        controller.view.frame = NSRect(x: 0, y: 0, width: 1280, height: 820); controller.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(controller.pageBadgeText, "1 / 1")
        XCTAssertGreaterThan(controller.thumbnailSidebarFrame.width, 160)
        XCTAssertTrue(controller.toolbarAccessibilityLabels.contains("批注"))
        XCTAssertTrue(controller.toolbarAccessibilityLabels.contains("缩小"))
        controller.toggleSidebarForTesting()
        controller.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(controller.thumbnailSidebarFrame.width, 0, accuracy: 0.5)
        XCTAssertGreaterThan(controller.pdfView.frame.width, 1200)
    }

    private func temporaryDirectory() -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try! FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }

    @MainActor func testDashboardResizesAutomaticallyAndContainsEveryCard() {
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: temporaryDirectory().appendingPathComponent("r.json")))
        let dashboard = shell.dashboardController
        _ = dashboard.view
        for width: CGFloat in [1280, 1180, 1040] {
            dashboard.view.frame = NSRect(x: 0, y: 0, width: width, height: 780)
            dashboard.select(.tools)
            dashboard.view.layoutSubtreeIfNeeded()
            XCTAssertEqual(dashboard.toolCardFrames.count, 28)
            XCTAssertLessThanOrEqual(dashboard.contentColumnFrame.width, 880.5)
            for frame in dashboard.toolCardFrames {
                XCTAssertEqual(frame.width, 136, accuracy: 0.5)
                XCTAssertGreaterThanOrEqual(frame.minX, 216)
                XCTAssertLessThanOrEqual(frame.maxX, width)
            }
            let cards = dashboard.toolCardFrames
            XCTAssertEqual(cards[1].minX - cards[0].maxX, 8, accuracy: 0.5)
        }
        XCTAssertLessThan(dashboard.renderedColumnCount, 6)
    }

    @MainActor func testToolCardsHaveDedicatedVisibleIconAndNeutralLabel() throws {
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: temporaryDirectory().appendingPathComponent("r.json")))
        let dashboard = shell.dashboardController; _ = dashboard.view
        dashboard.view.frame = NSRect(x: 0, y: 0, width: 1280, height: 820)
        dashboard.select(.tools); dashboard.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(dashboard.toolCardIconCount, 28)
    }

    @MainActor func testShellRoutesCommandsAndRetainsGeneratedDocumentSelection() throws {
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: temporaryDirectory().appendingPathComponent("r.json")))
        let first = shell.createBlankDocument()
        let second = shell.createBlankDocument()
        shell.activate(tabID: first.id)
        XCTAssertTrue(shell.routeDocumentCommand(#selector(WorkbenchController.insertBlank)))
        XCTAssertEqual(first.controller.workspace.document?.pageCount, 2)
        XCTAssertEqual(second.controller.workspace.document?.pageCount, 1)
        shell.showHome()
        XCTAssertFalse(shell.routeDocumentCommand(#selector(WorkbenchController.insertBlank)))
        shell.activate(tabID: second.id)
        XCTAssertTrue(shell.routeDocumentCommand(#selector(WorkbenchController.rotatePage)))
        XCTAssertEqual(second.controller.workspace.document?.page(at: 0)?.rotation, 90)
        XCTAssertEqual(first.controller.workspace.document?.page(at: 0)?.rotation, 0)
    }

    @MainActor func testReaderToolbarGroupsFitAndCenterAtSupportedSizes() throws {
        let controller = WorkbenchController(); _ = controller.view
        try controller.workspace.createBlank(); controller.refreshDocument()
        controller.view.translatesAutoresizingMaskIntoConstraints = false
        let widthConstraint = controller.view.widthAnchor.constraint(equalToConstant: 1280)
        widthConstraint.isActive = true
        controller.view.heightAnchor.constraint(equalToConstant: 780).isActive = true
        for width: CGFloat in [1280, 1180] {
            widthConstraint.constant = width
            controller.view.frame = NSRect(x: 0, y: 0, width: width, height: 780)
            controller.view.needsLayout = true
            controller.view.layoutSubtreeIfNeeded()
            XCTAssertEqual(controller.view.frame.width, width)
            let frames = controller.toolbarGroupFrames
            XCTAssertEqual(frames.count, 3)
            XCTAssertLessThanOrEqual(frames[0].maxX, frames[1].minX)
            XCTAssertLessThanOrEqual(frames[1].maxX, frames[2].minX)
            XCTAssertEqual(frames[1].midX, width / 2, accuracy: 2)
            XCTAssertLessThanOrEqual(frames[2].maxX, width)
        }
    }

    private func whiteImage() -> NSImage {
        let image = NSImage(size: NSSize(width: 100, height: 100)); image.lockFocus(); NSColor.white.setFill(); NSRect(x: 0, y: 0, width: 100, height: 100).fill(); image.unlockFocus(); return image
    }
}
