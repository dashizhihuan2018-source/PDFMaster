import AppKit
import XCTest
@testable import PaperDesk

@MainActor final class WindowCloseRoutingTests: XCTestCase {
    func testCloseCommandClosesFocusedSecondaryWindowWithoutClosingDocument() {
        let main = NSWindow(contentRect: .zero, styleMask: [.titled, .closable], backing: .buffered, defer: false)
        let secondary = CloseRecordingWindow(contentRect: .zero, styleMask: [.titled, .closable], backing: .buffered, defer: false)
        var tabCloses = 0
        WindowCloseRouting.perform(keyWindow: secondary, mainWindow: main) { tabCloses += 1 }
        XCTAssertEqual(secondary.closeRequests, 1)
        XCTAssertEqual(tabCloses, 0)
    }

    func testCloseCommandOnMainWindowClosesCurrentTab() {
        let main = NSWindow(contentRect: .zero, styleMask: [.titled, .closable], backing: .buffered, defer: false)
        var tabCloses = 0
        WindowCloseRouting.perform(keyWindow: main, mainWindow: main) { tabCloses += 1 }
        XCTAssertEqual(tabCloses, 1)
    }

    func testCloseCommandDoesNotCloseDocumentBehindNonClosableWindow() {
        let main = NSWindow(contentRect: .zero, styleMask: [.titled, .closable], backing: .buffered, defer: false)
        let auxiliary = CloseRecordingWindow(contentRect: .zero, styleMask: [.titled], backing: .buffered, defer: false)
        var tabCloses = 0
        WindowCloseRouting.perform(keyWindow: auxiliary, mainWindow: main) { tabCloses += 1 }
        XCTAssertEqual(auxiliary.closeRequests, 0)
        XCTAssertEqual(tabCloses, 0)
    }
}

@MainActor private final class CloseRecordingWindow: NSWindow {
    var closeRequests = 0
    override func performClose(_ sender: Any?) { closeRequests += 1 }
}
