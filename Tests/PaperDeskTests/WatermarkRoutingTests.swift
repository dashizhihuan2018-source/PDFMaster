import AppKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor final class WatermarkRoutingTests: XCTestCase {
    func testWatermarkIsNotAHomeCardAndCannotOpenAnotherDocument() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: directory.appendingPathComponent("recent.json")))
        let first = shell.createBlankDocument()
        let second = shell.createBlankDocument()
        shell.activate(tabID: first.id)
        XCTAssertFalse(shell.dispatch(toolID: "remove-watermark", openProvider: { XCTFail("Removed card must not request another document"); return nil }))
        XCTAssertTrue(shell.activeWorkbench === first.controller)
        XCTAssertFalse(shell.activeWorkbench === second.controller)
        XCTAssertEqual(shell.documentTabs.count, 2)
    }

    func testWatermarkMenuRequiresDocumentAndNoRunningLocalJob() throws {
        let workbench = WorkbenchController(); _ = workbench.view
        let item = NSMenuItem(title: "删除水印", action: #selector(WorkbenchController.removeWatermark(_:)), keyEquivalent: "")
        XCTAssertFalse(workbench.validateMenuItem(item))
        try workbench.workspace.createBlank()
        XCTAssertTrue(workbench.validateMenuItem(item))
        workbench.localToolCancellation = LocalToolCancellation()
        XCTAssertFalse(workbench.validateMenuItem(item))
        workbench.localToolCancellation = nil
    }

    func testFileMenuExposesFocusedWatermarkCommand() throws {
        _ = NSApplication.shared
        let oldMenu = NSApp.mainMenu
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: directory.appendingPathComponent("recent.json")))
        let delegate = AppDelegate(controller: shell)
        delegate.applicationDidFinishLaunching(Notification(name: NSApplication.didFinishLaunchingNotification))
        defer {
            for session in delegate.windowSessions { session.window.delegate = nil; session.window.close() }
            NSApp.mainMenu = oldMenu
            try? FileManager.default.removeItem(at: directory)
        }
        let menu = try XCTUnwrap(NSApp.mainMenu?.items.first { $0.submenu?.title == "文件" }?.submenu)
        let command = try XCTUnwrap(menu.items.first { $0.title == "删除水印" })
        XCTAssertFalse(delegate.validateMenuItem(command))
        _ = shell.createBlankDocument()
        XCTAssertTrue(delegate.validateMenuItem(command))
    }
}
