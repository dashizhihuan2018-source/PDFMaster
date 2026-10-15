import AppKit
import XCTest
@testable import PaperDesk

final class WindowChromeTests: XCTestCase {
    func testReferencePaletteDoesNotShiftBlueAndDarkGreysThroughCalibratedGamma() throws {
        let blue = try XCTUnwrap(ShellPalette.accent.usingColorSpace(.sRGB))
        XCTAssertEqual(blue.redComponent, 10.0 / 255, accuracy: 0.002)
        XCTAssertEqual(blue.greenComponent, 120.0 / 255, accuracy: 0.002)
        let canvas = try XCTUnwrap(ShellPalette.canvas.usingColorSpace(.sRGB))
        XCTAssertEqual(canvas.redComponent, 18.0 / 255, accuracy: 0.002)
    }
    @MainActor func testUnifiedChromeUsesRealWindowButtonsAndIconTabs() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: directory.appendingPathComponent("recent.json")))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820), styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
        window.titleVisibility = .hidden; window.titlebarAppearsTransparent = true
        window.contentViewController = shell
        let originalButtons = [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton].compactMap { window.standardWindowButton($0) }
        shell.installWindowChrome(in: window)
        shell.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(shell.nativeWindowControlCount, 3)
        XCTAssertEqual(shell.chromeHeight, 40)
        XCTAssertTrue(shell.homeTabIsIconOnly)
        XCTAssertTrue(shell.newTabIsBorderless)
        shell.dashboardController.select(.tools)
        shell.activateNewTabForTesting()
        XCTAssertEqual(shell.dashboardController.selectedSection, .home)
        for button in originalButtons {
            XCTAssertTrue(button.window === window)
            XCTAssertEqual(button.frame.height, 14, accuracy: 0.5)
            let frame = shell.view.convert(button.bounds, from: button)
            XCTAssertGreaterThanOrEqual(frame.minX, 10)
            XCTAssertLessThan(frame.maxX, 100)
            XCTAssertEqual(frame.midY, shell.view.bounds.height - 20, accuracy: 1)
        }
        shell.installWindowChrome(in: window)
        XCTAssertEqual(shell.nativeWindowControlCount, 3)
        let first = shell.createBlankDocument()
        let second = shell.createBlankDocument()
        XCTAssertEqual(shell.highlightedDocumentTabID, second.id)
        shell.activate(tabID: first.id)
        XCTAssertEqual(shell.highlightedDocumentTabID, first.id)
        shell.showHome()
        XCTAssertNil(shell.highlightedDocumentTabID)
        XCTAssertTrue(shell.homeTabIsIconOnly)
        XCTAssertTrue(shell.newTabIsBorderless)
    }
}
