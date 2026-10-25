import AppKit
import XCTest
@testable import PaperDesk

@MainActor final class CopilotSidebarTests: XCTestCase {
    private func shell() -> ApplicationShellController {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).appendingPathComponent("recent.json")
        return ApplicationShellController(recentStore: RecentDocumentStore(url: url))
    }
    func testNoDocumentDoesNotCreateAIWindow() {
        let shell = shell(); _ = shell.view
        XCTAssertFalse(shell.showCopilot()); XCTAssertNil(shell.copilotController)
    }
    func testAdvancedTextActionsRetainAnExplicitRunControl() throws {
        let shell = shell(); _ = shell.createBlankDocument(); XCTAssertTrue(shell.showCopilot())
        let c = try XCTUnwrap(shell.copilotController)
        func buttons(_ view: NSView) -> [NSButton] { (view as? NSButton).map { [$0] } ?? view.subviews.flatMap(buttons) }
        let advanced = try XCTUnwrap(c.actionChoice.superview)
        XCTAssertTrue(buttons(advanced).contains { $0.action.map(NSStringFromSelector) == "generate" }, "Explain and translate must not become unreachable when the composer sends questions")
        shell.hideCopilot()
    }
    func testCopilotIsEmbeddedOnReaderRightNotSeparateWindow() throws {
        let shell = shell(); let tab = shell.createBlankDocument()
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentViewController = shell; defer { window.close() }
        shell.view.layoutSubtreeIfNeeded()
        XCTAssertTrue(shell.showCopilot())
        let copilot = try XCTUnwrap(shell.copilotController)
        XCTAssertNil(copilot.window, "Copilot must not own a floating window")
        XCTAssertTrue(copilot.panelView.isDescendant(of: tab.controller.view))
        XCTAssertTrue(tab.controller.isCopilotVisible)
        for size in [NSSize(width: 1280, height: 820), NSSize(width: 1180, height: 700)] {
            window.setContentSize(size); shell.view.layoutSubtreeIfNeeded()
            let panel = copilot.panelView.convert(copilot.panelView.bounds, to: tab.controller.view)
            let pdf = tab.controller.pdfView.convert(tab.controller.pdfView.bounds, to: tab.controller.view)
            XCTAssertGreaterThanOrEqual(panel.width, 340)
            XCTAssertGreaterThan(pdf.width, 450)
            XCTAssertLessThanOrEqual(pdf.maxX, panel.minX + 1)
            XCTAssertEqual(panel.maxX, tab.controller.view.bounds.maxX, accuracy: 1)
        }
        shell.hideCopilot()
    }
    func testHeaderBodyAndComposerMatchReferenceOrderAtNarrowWidth() throws {
        let shell = shell(); _ = shell.createBlankDocument()
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1180, height: 700), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentViewController = shell; window.setContentSize(NSSize(width: 1180, height: 700)); defer { window.close() }
        shell.view.layoutSubtreeIfNeeded()
        XCTAssertTrue(shell.showCopilot()); shell.view.layoutSubtreeIfNeeded()
        let c = try XCTUnwrap(shell.copilotController)
        c.panelView.layoutSubtreeIfNeeded()
        let input = c.questionField.convert(c.questionField.bounds, to: c.panelView)
        let output = c.answerView.enclosingScrollView!.convert(c.answerView.enclosingScrollView!.bounds, to: c.panelView)
        let close = c.closeButton.convert(c.closeButton.bounds, to: c.panelView)
        XCTAssertGreaterThan(close.minY, output.maxY - 1)
        XCTAssertGreaterThan(output.minY, input.maxY)
        XCTAssertGreaterThan(output.height, 200, "shell=\(shell.view.frame) bench=\(shell.activeWorkbench!.view.frame) host=\(c.panelView.superview!.frame) panel=\(c.panelView.frame), children=\(c.panelView.subviews.map { String(describing: type(of: $0)) + String(describing: $0.frame) })")
        XCTAssertGreaterThan(input.width, 200)
        XCTAssertEqual(c.quickActionTitles, ["总结摘要", "列出要点", "批注摘要", "生成小测"])
        XCTAssertEqual(c.consentButton.state, .off)
        XCTAssertFalse(c.modelField.isDescendant(of: c.panelView), "Advanced configuration belongs in overflow, not the chat feed")
        shell.hideCopilot()
    }
    func testCloseButtonRevokesConsentAndRestoresReaderWidth() throws {
        let shell = shell(); let tab = shell.createBlankDocument()
        shell.view.layoutSubtreeIfNeeded(); let original = tab.controller.pdfView.frame.width
        XCTAssertTrue(shell.showCopilot()); shell.view.layoutSubtreeIfNeeded()
        let c = try XCTUnwrap(shell.copilotController); c.consentButton.state = .on
        c.closeButton.performClick(nil); shell.view.layoutSubtreeIfNeeded()
        XCTAssertNil(shell.copilotController); XCTAssertFalse(tab.controller.isCopilotVisible)
        XCTAssertNil(c.panelView.superview); XCTAssertEqual(c.consentButton.state, .off)
        XCTAssertEqual(tab.controller.pdfView.frame.width, original, accuracy: 1)
    }
    func testToolbarReentryTogglesSidebarWithoutDuplicateWindow() throws {
        let shell = shell(); let tab = shell.createBlankDocument()
        XCTAssertTrue(shell.showCopilot()); let c = try XCTUnwrap(shell.copilotController)
        XCTAssertTrue(shell.showCopilot())
        XCTAssertFalse(tab.controller.isCopilotVisible); XCTAssertNil(shell.copilotController)
        XCTAssertNil(c.panelView.superview)
        XCTAssertTrue(shell.showCopilot()); XCTAssertTrue(tab.controller.isCopilotVisible)
        shell.hideCopilot()
    }
    func testSwitchingTabsDetachesOldPanelAndClearsAnswer() throws {
        let shell = shell(); let first = shell.createBlankDocument(); XCTAssertTrue(shell.showCopilot())
        let c = try XCTUnwrap(shell.copilotController); c.answerView.string = "Private old answer"; c.consentButton.state = .on
        let second = shell.createBlankDocument()
        XCTAssertFalse(first.controller.isCopilotVisible); XCTAssertFalse(second.controller.isCopilotVisible)
        XCTAssertEqual(c.answerView.string, ""); XCTAssertEqual(c.consentButton.state, .off)
        XCTAssertNil(c.panelView.superview); XCTAssertNil(shell.copilotController)
    }
}
