import AppKit
import XCTest
@testable import PaperDesk

@MainActor final class TitlebarMaximizeTests: XCTestCase {
    // Hidden windows resize normally; entering a fullscreen Space is forbidden.
    private final class RecordingWindow: NSWindow {
        var fullscreenRequests = 0
        var dragRequests = 0
        override func toggleFullScreen(_ sender: Any?) { fullscreenRequests += 1 }
        override func performDrag(with event: NSEvent) { dragRequests += 1 }
    }

    func testDoubleClickFillsCurrentScreenVisibleAreaAndRestoresAfterTabRebuilds() throws {
        try withShell { shell, window in
            let first = shell.createBlankDocument()
            _ = shell.createBlankDocument()
            for width: CGFloat in [1180, 1600, 3008] {
                window.setContentSize(NSSize(width: width, height: 820))
                shell.activate(tabID: first.id)
                shell.view.layoutSubtreeIfNeeded()
                let blank = try XCTUnwrap(descendants(shell.view).compactMap { $0 as? WindowTitlebarBlankSpace }.first)
                let bar = try XCTUnwrap(blank.superview as? WindowTitlebarStackView)
                let available = NSRect(x: -1920, y: 40, width: 1920, height: 1040)
                bar.visibleScreenFrame = { _ in available }
                XCTAssertGreaterThan(blank.bounds.width, 20)
                let point = blank.convert(NSPoint(x: blank.bounds.midX, y: blank.bounds.midY), to: nil)
                let original = window.frame
                let target: NSView = width == 1600 ? try XCTUnwrap(blank.superview) : blank
                target.mouseDown(with: try mouse(window, point: point))
                XCTAssertEqual(window.frame, available)
                XCTAssertEqual(window.fullscreenRequests, 0)
                XCTAssertFalse(window.isVisible)
                XCTAssertFalse(window.styleMask.contains(.fullScreen))
                shell.showHome()
                shell.view.layoutSubtreeIfNeeded()
                let updatedBlank = try XCTUnwrap(bar.blankSpace)
                let restorePoint = updatedBlank.convert(NSPoint(x: updatedBlank.bounds.midX, y: updatedBlank.bounds.midY), to: nil)
                updatedBlank.mouseDown(with: try mouse(window, point: restorePoint))
                XCTAssertEqual(window.frame, original)
            }
        }
    }

    func testTitlebarControlsTabsAndReaderDoNotMaximize() throws {
        try withShell { shell, window in
            _ = shell.createBlankDocument()
            shell.view.layoutSubtreeIfNeeded()
            let bar = try XCTUnwrap(descendants(shell.view).compactMap { $0 as? WindowTitlebarStackView }.first)
            let original = window.frame
            let buttons = descendants(shell.view).compactMap { $0 as? NSButton }.filter {
                ["主界面标签", "新标签"].contains($0.accessibilityLabel() ?? "") ||
                ($0.accessibilityLabel()?.hasPrefix("文档标签 ") == true) ||
                ($0.accessibilityLabel()?.hasPrefix("关闭 ") == true)
            }
            XCTAssertGreaterThanOrEqual(buttons.count, 4)
            for button in buttons + [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton].compactMap({ window.standardWindowButton($0) }) {
                let point = button.convert(NSPoint(x: button.bounds.midX, y: button.bounds.midY), to: nil)
                XCTAssertFalse(bar.handleBackgroundMouseDown(try mouse(window, point: point)))
            }
            let below = bar.convert(NSPoint(x: bar.bounds.midX, y: -20), to: nil)
            XCTAssertFalse(bar.handleBackgroundMouseDown(try mouse(window, point: below)))
            XCTAssertEqual(window.fullscreenRequests, 0)
            XCTAssertEqual(window.frame, original)
        }
    }

    func testResizeAfterMaximizeUsesNewFrameForRestoreAndMissingScreenDoesNothing() throws {
        try withShell { shell, window in
            shell.view.layoutSubtreeIfNeeded()
            let bar = try XCTUnwrap(descendants(shell.view).compactMap { $0 as? WindowTitlebarStackView }.first)
            let available = NSRect(x: 2000, y: 30, width: 1920, height: 1040)
            bar.visibleScreenFrame = { _ in available }
            @MainActor func doubleClick() throws -> Bool {
                shell.view.layoutSubtreeIfNeeded()
                let blank = try XCTUnwrap(bar.blankSpace)
                return bar.handleBackgroundMouseDown(try mouse(window, point: blank.convert(NSPoint(x: blank.bounds.midX, y: blank.bounds.midY), to: nil)))
            }
            XCTAssertTrue(try doubleClick())
            XCTAssertEqual(window.frame, available)
            let resized = NSRect(x: 2100, y: 100, width: 1280, height: 820)
            window.setFrame(resized, display: false)
            XCTAssertTrue(try doubleClick())
            XCTAssertEqual(window.frame, available)
            XCTAssertTrue(try doubleClick())
            XCTAssertEqual(window.frame, resized)
            bar.visibleScreenFrame = { _ in nil }
            XCTAssertFalse(try doubleClick())
            XCTAssertEqual(window.frame, resized)
            XCTAssertEqual(window.fullscreenRequests, 0)
            XCTAssertFalse(window.isVisible)
        }
    }

    func testSingleClickDragsWhileTripleRightAndOtherWindowClicksAreIgnored() throws {
        try withShell { shell, window in
            shell.view.layoutSubtreeIfNeeded()
            let bar = try XCTUnwrap(descendants(shell.view).compactMap { $0 as? WindowTitlebarStackView }.first)
            let blank = try XCTUnwrap(bar.blankSpace)
            let point = blank.convert(NSPoint(x: blank.bounds.midX, y: blank.bounds.midY), to: nil)
            blank.mouseDown(with: try mouse(window, point: point, count: 1))
            bar.mouseDown(with: try mouse(window, point: point, count: 1))
            XCTAssertEqual(window.dragRequests, 2)
            XCTAssertFalse(bar.handleBackgroundMouseDown(try mouse(window, point: point, count: 3)))
            XCTAssertFalse(bar.handleBackgroundMouseDown(try mouse(window, point: point, type: .rightMouseDown)))
            let foreign = RecordingWindow(contentRect: NSRect(x: 0, y: 0, width: 200, height: 100), styleMask: [.titled], backing: .buffered, defer: false)
            foreign.isReleasedWhenClosed = false
            defer { foreign.close() }
            XCTAssertFalse(bar.handleBackgroundMouseDown(try mouse(foreign, point: point)))
            XCTAssertEqual(window.fullscreenRequests, 0)
            XCTAssertFalse(bar.mouseDownCanMoveWindow)
            XCTAssertFalse(blank.mouseDownCanMoveWindow)
        }
    }

    private func mouse(_ window: NSWindow, point: NSPoint, count: Int = 2, type: NSEvent.EventType = .leftMouseDown) throws -> NSEvent {
        try XCTUnwrap(NSEvent.mouseEvent(with: type, location: point, modifierFlags: [], timestamp: 0,
                                       windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: count, pressure: 1))
    }
    private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap { descendants($0) } }
    private func withShell(_ body: (ApplicationShellController, RecordingWindow) throws -> Void) throws {
        _ = NSApplication.shared
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: root.appendingPathComponent("recent.json")))
        let window = RecordingWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820),
                                     styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = shell
        shell.installWindowChrome(in: window)
        defer { window.contentViewController = nil; window.close(); try? FileManager.default.removeItem(at: root) }
        try body(shell, window)
    }
}
