import AppKit
import XCTest
@testable import PaperDesk

@MainActor final class BackgroundTestPolicyTests: XCTestCase {
    func testForegroundRequiresExactExplicitOptIn() {
        for value in [nil, "", "0", "true", "yes"] as [String?] {
            let environment = value.map { ["PDFMASTER_TEST_ALLOW_FOREGROUND": $0] } ?? [:]
            XCTAssertFalse(NativeTestEnvironment.allowsForeground(environment: environment))
        }
        XCTAssertTrue(NativeTestEnvironment.allowsForeground(environment: ["PDFMASTER_TEST_ALLOW_FOREGROUND": "1"]))
    }

    func testBackgroundPresentationNeverOrdersOrKeysWindow() {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 100, height: 100), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        ApplicationPresentation.background.present(window, makeKey: true, activate: true)
        ApplicationPresentation.background.present(window, makeKey: false)
        XCTAssertFalse(window.isVisible); XCTAssertFalse(window.isKeyWindow)
    }

    func testBackgroundDelegateKeepsNewTransferredAndReopenedWindowsHidden() throws {
        _ = NSApplication.shared
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: root.appendingPathComponent("recent.json")))
        let delegate = AppDelegate(controller: shell, presentation: .background)
        let menu = NSApp.mainMenu
        defer {
            for session in delegate.windowSessions {
                session.window.delegate = nil; session.window.contentViewController = nil; session.window.close()
            }
            NSApp.mainMenu = menu; try? FileManager.default.removeItem(at: root)
        }
        delegate.applicationDidFinishLaunching(Notification(name: NSApplication.didFinishLaunchingNotification))
        let tab = shell.createBlankDocument()
        XCTAssertTrue(delegate.moveTabToNewWindow(tab.id, from: shell))
        _ = delegate.newWindow(makeKey: false)
        XCTAssertTrue(delegate.applicationShouldHandleReopen(NSApp, hasVisibleWindows: false))
        let source = root.appendingPathComponent("open.pdf")
        try tab.controller.workspace.saveCopy(to: source)
        XCTAssertTrue(delegate.application(NSApp, openFile: source.path))
        XCTAssertEqual(delegate.windowSessions.count, 3)
        XCTAssertTrue(delegate.windowSessions.allSatisfy { !$0.window.isVisible && !$0.window.isKeyWindow })
    }

    func testKnownTestPresentationEntryPointsHaveExplicitGuards() throws {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let process = Process(), output = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = ["python3", root.appendingPathComponent("scripts/check-background-tests.py").path]
        process.standardOutput = output; process.standardError = output
        try process.run()
        let data = output.fileHandleForReading.readDataToEndOfFile(); process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0, String(decoding: data, as: UTF8.self))
    }
}
