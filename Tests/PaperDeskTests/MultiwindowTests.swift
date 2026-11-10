import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor
final class MultiwindowTests: XCTestCase {
    func testNewWindowCreatesIndependentShellAndRoutesByWindow() throws {
        try withDelegate { delegate, first in
            let firstTab = first.createBlankDocument()
            let secondSession = delegate.newWindow(makeKey: false)

            XCTAssertEqual(delegate.windowSessions.count, 2)
            XCTAssertFalse(first === secondSession.controller)
            XCTAssertEqual(first.documentTabs.map(\.id), [firstTab.id])
            XCTAssertTrue(secondSession.controller.documentTabs.isEmpty)
            XCTAssertTrue(delegate.controller(for: secondSession.window) === secondSession.controller)
            XCTAssertTrue(delegate.controller(for: nil) === first)
        }
    }

    func testMoveTabPreservesControllerDirtyPageZoomAndUndoIdentity() throws {
        try withDelegate { delegate, source in
            let tab = source.createBlankDocument()
            let before = try XCTUnwrap(source.activeWorkbench)
            try before.workspace.insertBlank(at: 1)
            before.refreshDocument(preservePage: 1, scale: 1.75)
            let revision = before.workspace.revision

            XCTAssertTrue(before.workspace.isDirty)
            XCTAssertTrue(before.workspace.canUndo)
            XCTAssertEqual(before.copilotPageIndex, 1)
            XCTAssertEqual(before.pdfView.scaleFactor, 1.75, accuracy: 0.001)

            XCTAssertTrue(delegate.moveTabToNewWindow(tab.id, from: source))
            let destination = try XCTUnwrap(delegate.windowSessions.last?.controller)
            let moved = try XCTUnwrap(destination.documentTabs.first)

            XCTAssertTrue(moved === tab)
            XCTAssertTrue(destination.activeWorkbench === before)
            XCTAssertFalse(source.documentTabs.contains { $0.id == tab.id })
            XCTAssertEqual(before.workspace.revision, revision)
            XCTAssertTrue(before.workspace.isDirty)
            XCTAssertTrue(before.workspace.canUndo)
            XCTAssertEqual(before.copilotPageIndex, 1)
            XCTAssertEqual(before.pdfView.scaleFactor, 1.75, accuracy: 0.001)
        }
    }

    func testTransferredTabRebindsReaderCommandsToDestinationShell() throws {
        try withDelegate { delegate, source in
            let moving = source.createBlankDocument()
            _ = source.createBlankDocument()
            source.activate(tabID: moving.id)
            let beforeItem = moving.controller.readerCommands
                .makeMenu(commands: [.closeOtherTabs], title: "test").items[0]
            XCTAssertTrue(moving.controller.readerCommands.validateMenuItem(beforeItem))

            XCTAssertTrue(delegate.moveTabToNewWindow(moving.id, from: source))

            let afterItem = moving.controller.readerCommands
                .makeMenu(commands: [.closeOtherTabs], title: "test").items[0]
            XCTAssertFalse(moving.controller.readerCommands.validateMenuItem(afterItem),
                           "The transferred controller must no longer consult the source shell's second tab")
            XCTAssertEqual(source.documentTabs.count, 1)
            XCTAssertEqual(delegate.windowSessions.last?.controller.documentTabs.map(\.id), [moving.id])
        }
    }

    func testWindowCloseGuardsOnlyThatWindowButQuitGuardsEveryWindow() throws {
        try withDelegate { delegate, first in
            let firstTab = first.createBlankDocument()
            try firstTab.controller.workspace.addPopupNote("first", author: nil, page: 0,
                bounds: CGRect(x: 20, y: 20, width: 20, height: 20), color: .yellow)
            let second = delegate.newWindow(makeKey: false)
            let secondTab = second.controller.createBlankDocument()
            try secondTab.controller.workspace.addPopupNote("second", author: nil, page: 0,
                bounds: CGRect(x: 30, y: 30, width: 20, height: 20), color: .yellow)
            var closeAsked: [UUID] = []

            XCTAssertTrue(delegate.canCloseWindow(second.window, documentDecision: {
                closeAsked.append($0.id); return true
            }))
            XCTAssertEqual(closeAsked, [secondTab.id])

            var quitAsked: [UUID] = []
            XCTAssertFalse(delegate.canTerminate(documentDecision: {
                quitAsked.append($0.id); return $0.id != secondTab.id
            }))
            XCTAssertEqual(Set(quitAsked), Set([firstTab.id, secondTab.id]))
        }
    }

    func testClosingOneOfTwoWindowsRemovesOnlyThatSession() throws {
        try withDelegate { delegate, first in
            let firstWindow = try XCTUnwrap(delegate.windowSessions.first?.window)
            let second = delegate.newWindow(makeKey: false)

            second.window.close()
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))

            XCTAssertEqual(delegate.windowSessions.count, 1)
            XCTAssertTrue(delegate.windowSessions[0].window === firstWindow)
            XCTAssertTrue(delegate.windowSessions[0].controller === first)
        }
    }

    func testDragPolicyRequiresThresholdAndOutsideDropAndHonorsEscape() {
        XCTAssertFalse(TabDragPolicy.hasCrossedThreshold(from: .zero, to: NSPoint(x: 3, y: 4)))
        XCTAssertTrue(TabDragPolicy.hasCrossedThreshold(from: .zero, to: NSPoint(x: 7, y: 0)))
    }

    func testDragTrackerInvokesTransferOnlyForThresholdedOutsideMouseUp() {
        let id = UUID()
        var transferred: [UUID] = []
        let tracker = TabDragTracker(tabID: id, start: .zero,
                                     isInsideTabBar: { $0.x < 100 },
                                     onTransfer: { transferred.append($0); return true })
        XCTAssertFalse(tracker.update(to: NSPoint(x: 3, y: 4)))
        XCTAssertFalse(tracker.finish(at: NSPoint(x: 150, y: 0)))
        XCTAssertTrue(transferred.isEmpty)

        let outside = TabDragTracker(tabID: id, start: .zero,
                                     isInsideTabBar: { $0.x < 100 },
                                     onTransfer: { transferred.append($0); return true })
        XCTAssertTrue(outside.update(to: NSPoint(x: 7, y: 0)))
        XCTAssertTrue(outside.finish(at: NSPoint(x: 150, y: 0)))
        XCTAssertEqual(transferred, [id])
    }

    func testDragTrackerEscapeAndInsideDropNeverInvokeTransferCallback() {
        let id = UUID()
        var transfers = 0
        let cancelled = TabDragTracker(tabID: id, start: .zero,
                                       isInsideTabBar: { _ in false },
                                       onTransfer: { _ in transfers += 1; return true })
        XCTAssertTrue(cancelled.update(to: NSPoint(x: 8, y: 0)))
        cancelled.cancel()
        XCTAssertFalse(cancelled.finish(at: NSPoint(x: 200, y: 0)))

        let inside = TabDragTracker(tabID: id, start: .zero,
                                    isInsideTabBar: { _ in true },
                                    onTransfer: { _ in transfers += 1; return true })
        XCTAssertTrue(inside.update(to: NSPoint(x: 8, y: 0)))
        XCTAssertFalse(inside.finish(at: NSPoint(x: 20, y: 0)))
        XCTAssertEqual(transfers, 0)
    }

    func testFileMenuAndTabContextMenuExposeWindowCommands() throws {
        try withDelegate { delegate, shell in
            _ = shell.createBlankDocument()
            shell.view.layoutSubtreeIfNeeded()
            let fileMenu = try XCTUnwrap(NSApp.mainMenu?.items.first { $0.submenu?.title == "文件" }?.submenu)
            XCTAssertNotNil(fileMenu.items.first { $0.title == "新建窗口" })
            XCTAssertNotNil(fileMenu.items.first { $0.title == "将当前标签移到新窗口" })
            let newWindow = try XCTUnwrap(fileMenu.items.first { $0.title == "新建窗口" })
            let newCanvas = try XCTUnwrap(fileMenu.items.first { $0.title == "新建无边记" })
            XCTAssertEqual(shortcut(of: newWindow), Shortcut(key: "n", modifiers: [.command, .option]))
            XCTAssertEqual(shortcut(of: newCanvas), Shortcut(key: "n", modifiers: [.command, .shift]))
            let shortcuts = fileMenu.items.compactMap(shortcut)
            XCTAssertEqual(Set(shortcuts).count, shortcuts.count, "File menu shortcuts must be unique after case/modifier normalization")
            delegate.addFileMenuItem(NSMenuItem(title: "聚焦扩展命令", action: nil, keyEquivalent: ""))
            XCTAssertNotNil(fileMenu.items.first { $0.title == "聚焦扩展命令" })

            let tabButton = try XCTUnwrap(buttons(in: shell.view).first {
                $0.accessibilityLabel()?.hasPrefix("文档标签 ") == true
            })
            XCTAssertNotNil(tabButton.menu?.items.first { $0.title == "移到新窗口" })
        }
    }

    func testClosingFirstSessionReleasesBootstrapShellWhileSecondLives() throws {
        _ = NSApplication.shared
        let previousMenu = NSApp.mainMenu
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("MultiwindowRelease-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        var delegate: AppDelegate?
        weak var releasedShell: ApplicationShellController?
        autoreleasepool {
            var shell: ApplicationShellController? = ApplicationShellController(
                recentStore: RecentDocumentStore(url: root.appendingPathComponent("recents.json")))
            releasedShell = shell
            delegate = AppDelegate(controller: shell!, presentation: .background)
            delegate!.applicationDidFinishLaunching(Notification(name: NSApplication.didFinishLaunchingNotification))
            _ = delegate!.newWindow(makeKey: false)
            let firstWindow = delegate!.windowSessions[0].window
            shell = nil
            firstWindow.close()
        }
        RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        XCTAssertNil(releasedShell)
        if let delegate {
            for session in delegate.windowSessions {
                session.window.delegate = nil; session.window.contentViewController = nil
                session.window.orderOut(nil); session.window.close()
            }
        }
        delegate = nil
        NSApp.mainMenu = previousMenu
        try? FileManager.default.removeItem(at: root)
    }

    private func withDelegate(_ body: (AppDelegate, ApplicationShellController) throws -> Void) throws {
        _ = NSApplication.shared
        let previousMenu = NSApp.mainMenu
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("MultiwindowTests-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: root.appendingPathComponent("recents.json")))
        let delegate = AppDelegate(controller: shell, presentation: .background)
        delegate.applicationDidFinishLaunching(Notification(name: NSApplication.didFinishLaunchingNotification))
        defer {
            for session in delegate.windowSessions {
                session.window.delegate = nil
                session.window.contentViewController = nil
                session.window.orderOut(nil)
                session.window.close()
            }
            NSApp.mainMenu = previousMenu
            try? FileManager.default.removeItem(at: root)
        }
        try body(delegate, shell)
    }

    private func buttons(in view: NSView) -> [NSButton] {
        (view as? NSButton).map { [$0] } ?? view.subviews.flatMap { buttons(in: $0) }
    }

    private struct Shortcut: Hashable {
        let key: String
        let modifiers: NSEvent.ModifierFlags
        static func == (lhs: Shortcut, rhs: Shortcut) -> Bool {
            lhs.key == rhs.key && lhs.modifiers.rawValue == rhs.modifiers.rawValue
        }
        func hash(into hasher: inout Hasher) {
            hasher.combine(key); hasher.combine(modifiers.rawValue)
        }
    }

    private func shortcut(of item: NSMenuItem) -> Shortcut? {
        guard !item.keyEquivalent.isEmpty else { return nil }
        var modifiers = item.keyEquivalentModifierMask.intersection([.command, .option, .control, .shift])
        let lowered = item.keyEquivalent.lowercased()
        if item.keyEquivalent != lowered { modifiers.insert(.shift) }
        return Shortcut(key: lowered, modifiers: modifiers)
    }
}
