import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor final class ReaderLayoutOptionsTests: XCTestCase {
    private func button(_ label: String, in view: NSView) -> NSButton? {
        if let button = view as? NSButton, button.accessibilityLabel() == label { return button }
        for child in view.subviews {
            if let match = button(label, in: child) { return match }
        }
        return nil
    }

    func testToolbarLayoutButtonUsesPersistentSidebarAndTogglesOriginalContent() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument()
        XCTAssertTrue(c.readerCommands.run(.outline))
        let layoutButton = try XCTUnwrap(button("布局选项", in: c.view))

        layoutButton.performClick(nil)
        c.view.layoutSubtreeIfNeeded()

        XCTAssertTrue(c.isLayoutSidebarVisible)
        XCTAssertTrue(c.readerSidebar.view.isHidden)
        XCTAssertEqual(c.layoutSidebarFrame, c.thumbnailSidebarFrame)
        XCTAssertNil(c.view.window?.attachedSheet)
        XCTAssertEqual(c.readerSidebar.mode, .outline)

        layoutButton.performClick(nil)
        c.view.layoutSubtreeIfNeeded()

        XCTAssertFalse(c.isLayoutSidebarVisible)
        XCTAssertFalse(c.readerSidebar.view.isHidden)
        XCTAssertEqual(c.readerSidebar.mode, .outline)
    }

    func testLayoutSidebarRevealsFromHiddenAndOverviewAndReaderCommandsDismissIt() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument()
        let layoutButton = try XCTUnwrap(button("布局选项", in: c.view))
        let sidebarButton = try XCTUnwrap(button("侧栏", in: c.view))

        sidebarButton.performClick(nil)
        XCTAssertFalse(c.readerCommands.sidebarVisible)
        layoutButton.performClick(nil)
        XCTAssertTrue(c.isLayoutSidebarVisible)
        XCTAssertTrue(c.readerCommands.sidebarVisible)

        sidebarButton.performClick(nil)
        XCTAssertFalse(c.readerCommands.sidebarVisible)
        sidebarButton.performClick(nil)
        XCTAssertTrue(c.readerCommands.sidebarVisible)
        XCTAssertTrue(c.isLayoutSidebarVisible)
        XCTAssertTrue(c.readerSidebar.view.isHidden)

        sidebarButton.performClick(nil)
        XCTAssertFalse(c.readerCommands.sidebarVisible)
        layoutButton.performClick(nil)
        XCTAssertTrue(c.readerCommands.sidebarVisible)
        XCTAssertTrue(c.isLayoutSidebarVisible)
        XCTAssertTrue(c.readerSidebar.view.isHidden)

        XCTAssertTrue(c.readerCommands.run(.outline))
        XCTAssertFalse(c.isLayoutSidebarVisible)
        XCTAssertEqual(c.readerSidebar.mode, .outline)
        layoutButton.performClick(nil)
        XCTAssertTrue(c.readerCommands.run(.thumbnails))
        XCTAssertFalse(c.isLayoutSidebarVisible)
        XCTAssertEqual(c.readerSidebar.mode, .thumbnails)
        layoutButton.performClick(nil)
        XCTAssertTrue(c.readerCommands.run(.find))
        XCTAssertFalse(c.isLayoutSidebarVisible)
        XCTAssertEqual(c.readerSidebar.mode, .search)

        layoutButton.performClick(nil)
        XCTAssertTrue(c.isLayoutSidebarVisible)
        c.togglePageOverview()
        XCTAssertTrue(c.isOverviewVisible)
        layoutButton.performClick(nil)
        XCTAssertFalse(c.isOverviewVisible)
        XCTAssertTrue(c.isLayoutSidebarVisible)
    }

    func testLayoutSidebarStateIsIndependentPerWorkbench() throws {
        let a = WorkbenchController(), b = WorkbenchController(); _ = a.view; _ = b.view
        for c in [a, b] { try c.workspace.createBlank(); c.refreshDocument() }

        try XCTUnwrap(button("布局选项", in: a.view)).performClick(nil)

        XCTAssertTrue(a.isLayoutSidebarVisible)
        XCTAssertFalse(b.isLayoutSidebarVisible)
        XCTAssertFalse(b.readerSidebar.view.isHidden)
    }

    func testButtonsSwitchRealPDFViewAndCoverWithoutEditingDocument() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); try c.workspace.insertBlank(at: 1)
        try c.workspace.insertBlank(at: 2); c.refreshDocument()
        let before = try c.workspace.engineInput()
        let document = c.workspace.document
        let revision = c.workspace.revision, dirty = c.workspace.isDirty
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 1))
        c.pdfView.go(to: page); c.pdfView.autoScales = false; c.pdfView.scaleFactor = 1.37
        let originalPage = c.pdfView.currentPage
        let originalZoom = c.pdfView.scaleFactor
        let panel = ReaderLayoutOptionsController(commands: c.readerCommands); _ = panel.view
        XCTAssertEqual(panel.singleButton.state, .on)
        XCTAssertTrue(panel.singleButton.contentTintColor?.isEqual(NSColor.systemBlue) == true)
        XCTAssertTrue(panel.coverButton.isHidden)
        panel.doubleButton.performClick(nil)
        XCTAssertEqual(c.pdfView.displayMode, .twoUpContinuous)
        XCTAssertFalse(c.pdfView.displaysAsBook)
        XCTAssertEqual(panel.doubleButton.state, .on)
        XCTAssertTrue(panel.doubleButton.contentTintColor?.isEqual(NSColor.systemBlue) == true)
        XCTAssertFalse(panel.coverButton.isHidden)
        panel.coverButton.performClick(nil)
        XCTAssertTrue(c.pdfView.displaysAsBook)
        XCTAssertEqual(c.readerCommands.layout, .twoPagesWithCover)
        panel.coverButton.performClick(nil)
        XCTAssertFalse(c.pdfView.displaysAsBook)
        panel.continuousButton.performClick(nil)
        XCTAssertEqual(c.pdfView.displayMode, .twoUp)
        panel.singleButton.performClick(nil)
        XCTAssertEqual(c.pdfView.displayMode, .singlePage)
        XCTAssertTrue(panel.coverButton.isHidden)
        // PDFKit adds fresh /ID and timestamp fields on serialization; compare
        // the workspace's normalized content contract, not volatile PDF bytes.
        XCTAssertNoThrow(try c.workspace.validateEngineInput(before))
        XCTAssertTrue(c.workspace.document === document)
        XCTAssertTrue(c.pdfView.currentPage === originalPage)
        XCTAssertEqual(c.pdfView.scaleFactor, originalZoom, accuracy: 0.001)
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertEqual(c.workspace.isDirty, dirty)
    }

    func testFixedPanelAutomaticallyTracksCommandsAndKeepsOtherTabsUnchanged() throws {
        let a = WorkbenchController(), b = WorkbenchController(); _ = a.view; _ = b.view
        for c in [a, b] { try c.workspace.createBlank(); c.refreshDocument() }
        try XCTUnwrap(button("布局选项", in: a.view)).performClick(nil)
        let panel = a.layoutOptionsController
        XCTAssertTrue(a.readerCommands.run(.cover))
        XCTAssertEqual(panel.doubleButton.state, .on)
        XCTAssertEqual(panel.coverButton.state, .on)
        XCTAssertEqual(b.pdfView.displayMode, .singlePageContinuous)
        XCTAssertTrue(a.readerCommands.run(.singlePage))
        XCTAssertEqual(panel.singleButton.state, .on)
        XCTAssertTrue(panel.coverButton.isHidden)
    }

    func testDisplayMenuContinuousItemDispatchesWithoutEditingDocument() throws {
        _ = NSApplication.shared
        let previousMenu = NSApp.mainMenu
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ReaderContinuousMenu-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: root.appendingPathComponent("recents.json")))
        let tab = shell.createBlankDocument(), c = tab.controller
        try c.workspace.insertBlank(at: 1); try c.workspace.insertBlank(at: 2); c.refreshDocument()
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 1))
        c.pdfView.go(to: page); c.pdfView.autoScales = false; c.pdfView.scaleFactor = 1.37
        let before = try c.workspace.engineInput()
        let document = c.workspace.document, revision = c.workspace.revision, dirty = c.workspace.isDirty
        let originalPage = c.pdfView.currentPage, originalZoom = c.pdfView.scaleFactor, originalLayout = c.readerCommands.layout
        let delegate = AppDelegate(controller: shell)
        delegate.applicationDidFinishLaunching(Notification(name: NSApplication.didFinishLaunchingNotification))
        defer {
            for session in delegate.windowSessions {
                session.window.delegate = nil; session.window.contentViewController = nil
                session.window.orderOut(nil); session.window.close()
            }
            NSApp.mainMenu = previousMenu
            try? FileManager.default.removeItem(at: root)
        }
        let viewMenu = try XCTUnwrap(NSApp.mainMenu?.items.first { $0.identifier?.rawValue == "reader-view" }?.submenu)
        let continuous = try XCTUnwrap(viewMenu.items.first { $0.title == "连续滚动" })

        viewMenu.update()
        XCTAssertTrue(continuous.isEnabled)
        XCTAssertEqual(continuous.state, .on)
        XCTAssertTrue(NSApp.sendAction(try XCTUnwrap(continuous.action), to: continuous.target, from: continuous))
        viewMenu.update()

        XCTAssertFalse(c.readerCommands.continuous)
        XCTAssertEqual(continuous.state, .off)
        XCTAssertEqual(c.layoutOptionsController.continuousButton.state, .off)
        XCTAssertEqual(c.readerCommands.layout, originalLayout)
        XCTAssertTrue(c.pdfView.currentPage === originalPage)
        XCTAssertEqual(c.pdfView.scaleFactor, originalZoom, accuracy: 0.001)
        XCTAssertNoThrow(try c.workspace.validateEngineInput(before))
        XCTAssertTrue(c.workspace.document === document)
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertEqual(c.workspace.isDirty, dirty)

        c.togglePageOverview(); viewMenu.update()
        XCTAssertFalse(continuous.isEnabled)
        c.togglePageOverview()
        let empty = WorkbenchController(); _ = empty.view
        let noDocument = NSMenuItem(title: continuous.title, action: continuous.action, keyEquivalent: "")
        XCTAssertFalse(empty.validateMenuItem(noDocument))
    }

    func testPanelFitsMinimumSidebarWidthAndDisablesControlsWithoutDocument() {
        let session = ReaderSession(pdfView: PDFView())
        let commands = ReaderCommandController(session: session, sidebar: ReaderSidebarController(session: session))
        let panel = ReaderLayoutOptionsController(commands: commands); _ = panel.view
        panel.view.frame = NSRect(x: 0, y: 0, width: 176, height: 320)
        panel.view.layoutSubtreeIfNeeded()
        let singleFrame = panel.singleButton.convert(panel.singleButton.bounds, to: panel.view)
        let doubleFrame = panel.doubleButton.convert(panel.doubleButton.bounds, to: panel.view)

        XCTAssertLessThanOrEqual(doubleFrame.maxX, panel.view.bounds.maxX - 16 + 0.5)
        XCTAssertGreaterThanOrEqual(singleFrame.minX, panel.view.bounds.minX + 16 - 0.5)
        XCTAssertEqual(panel.singleButton.imagePosition, .imageLeading)
        XCTAssertEqual(panel.doubleButton.imagePosition, .imageLeading)
        XCTAssertTrue(panel.continuousButton.isHidden)
        XCTAssertTrue(panel.fitButton.isHidden)
        XCTAssertFalse(panel.singleButton.isEnabled)
        XCTAssertFalse(panel.doubleButton.isEnabled)
        XCTAssertFalse(panel.coverButton.isEnabled)
        XCTAssertFalse(panel.continuousButton.isEnabled)
        XCTAssertFalse(panel.fitButton.isEnabled)
    }
}
