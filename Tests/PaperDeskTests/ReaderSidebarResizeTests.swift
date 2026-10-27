import AppKit
import XCTest
@testable import PaperDesk

@MainActor final class ReaderSidebarResizeTests: XCTestCase {
    private var suite = ""
    private var defaults: UserDefaults!
    override func setUp() {
        super.setUp()
        suite = "PDFMaster.SidebarTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)!
    }
    override func tearDown() {
        defaults.removePersistentDomain(forName: suite)
        defaults = nil; super.tearDown()
    }
    private func findDivider(_ view: NSView) -> NSView? {
        if view.identifier?.rawValue == "reader-sidebar-divider" { return view }
        return view.subviews.lazy.compactMap { self.findDivider($0) }.first
    }

    func testSidebarProvidesDraggableBoundary() throws {
        let controller = WorkbenchController()
        controller.sidebarWidthPreference = ReaderSidebarWidth(defaults: defaults)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.contentViewController = controller
        defer { window.contentViewController = nil }
        controller.view.layoutSubtreeIfNeeded()
        let divider = try XCTUnwrap(findDivider(controller.view), "Native sidebar must expose a draggable right boundary")
        XCTAssertGreaterThanOrEqual(divider.bounds.width, 6)
        let initial = controller.thumbnailSidebarFrame.width
        let start = divider.convert(NSPoint(x: divider.bounds.midX, y: divider.bounds.midY), to: nil)
        func event(_ type: NSEvent.EventType, _ delta: CGFloat) -> NSEvent {
            NSEvent.mouseEvent(with: type, location: NSPoint(x: start.x + delta, y: start.y), modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
        }
        divider.mouseDown(with: event(.leftMouseDown, 0))
        divider.mouseDragged(with: event(.leftMouseDragged, 100))
        divider.mouseUp(with: event(.leftMouseUp, 100))
        controller.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(controller.thumbnailSidebarFrame.width, initial + 100, accuracy: 0.5)
        XCTAssertEqual(ReaderSidebarWidth(defaults: defaults).preferred, initial + 100)
    }

    func testPreferenceValidationAndTemporaryWindowClamping() {
        for saved in [Double.nan, Double.infinity, -1, 0, 175, 481] {
            defaults.set(saved, forKey: ReaderSidebarWidth.key)
            XCTAssertEqual(ReaderSidebarWidth(defaults: defaults).preferred, 176)
        }
        let preference = ReaderSidebarWidth(defaults: defaults)
        preference.commit(400)
        XCTAssertEqual(preference.fitted(available: 430), 230)
        XCTAssertEqual(preference.fitted(available: 250), 50)
        XCTAssertEqual(preference.fitted(available: 100), 0)
        XCTAssertEqual(preference.fitted(available: 1200), 400)
        XCTAssertEqual(ReaderSidebarWidth(defaults: defaults).preferred, 400)
        XCTAssertEqual(preference.fitted(999, available: 1200), 480)
        XCTAssertEqual(preference.fitted(-999, available: 1200), 176)
    }

    func testModesHideRestoreOverviewAndCopilotKeepPreferredWidthWithoutEditingPDF() throws {
        let c = WorkbenchController()
        c.sidebarWidthPreference = ReaderSidebarWidth(defaults: defaults)
        c.sidebarWidthPreference.commit(400)
        try c.workspace.createBlank(); c.refreshDocument()
        let revision = c.workspace.revision
        let dirty = c.workspace.isDirty
        let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        w.contentViewController = c; defer { w.contentViewController = nil }
        for mode: ReaderSidebarMode in [.thumbnails, .outline, .search, .annotations] {
            c.readerSidebar.show(mode); c.view.layoutSubtreeIfNeeded()
            XCTAssertEqual(c.thumbnailSidebarFrame.width, 400, accuracy: 0.5)
        }
        c.toggleSidebarForTesting(); c.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(c.thumbnailSidebarFrame.width, 0, accuracy: 0.5)
        XCTAssertTrue(try XCTUnwrap(findDivider(c.view)).isHidden)
        c.toggleSidebarForTesting(); c.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(c.thumbnailSidebarFrame.width, 400, accuracy: 0.5)
        c.togglePageOverview(); c.view.layoutSubtreeIfNeeded()
        XCTAssertTrue(try XCTUnwrap(findDivider(c.view)).isHidden)
        c.togglePageOverview(); c.view.layoutSubtreeIfNeeded()
        XCTAssertFalse(try XCTUnwrap(findDivider(c.view)).isHidden)
        w.setContentSize(NSSize(width: 700, height: 720))
        c.setCopilotPanel(NSView()); c.view.layoutSubtreeIfNeeded()
        XCTAssertGreaterThanOrEqual(c.pdfView.bounds.width, 200)
        XCTAssertLessThanOrEqual(c.thumbnailSidebarFrame.width + 200, c.pdfView.superview!.bounds.width + 0.5)
        c.setCopilotPanel(nil); w.setContentSize(NSSize(width: 1280, height: 820)); c.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(c.thumbnailSidebarFrame.width, 400, accuracy: 0.5)
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertEqual(c.workspace.isDirty, dirty)
        XCTAssertEqual(ReaderSidebarWidth(defaults: defaults).preferred, 400)
    }

    func testKeyboardAdjustmentEscapeAndFocusLossCancelDrag() throws {
        let c = WorkbenchController(); c.sidebarWidthPreference = ReaderSidebarWidth(defaults: defaults)
        let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        w.contentViewController = c; defer { w.contentViewController = nil }
        c.view.layoutSubtreeIfNeeded()
        let divider = try XCTUnwrap(findDivider(c.view))
        func key(_ code: UInt16) -> NSEvent {
            NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: w.windowNumber, context: nil, characters: "", charactersIgnoringModifiers: "", isARepeat: false, keyCode: code)!
        }
        divider.keyDown(with: key(124)); c.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(c.thumbnailSidebarFrame.width, 184, accuracy: 0.5)
        divider.keyDown(with: key(119)); c.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(c.thumbnailSidebarFrame.width, 480, accuracy: 0.5)
        divider.keyDown(with: key(115)); c.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(c.thumbnailSidebarFrame.width, 176, accuracy: 0.5)
        func mouse(_ type: NSEvent.EventType, _ x: CGFloat) -> NSEvent {
            NSEvent.mouseEvent(with: type, location: NSPoint(x: x, y: 300), modifierFlags: [], timestamp: 0, windowNumber: w.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
        }
        for cancel in [true, false] {
            divider.mouseDown(with: mouse(.leftMouseDown, 176))
            divider.mouseDragged(with: mouse(.leftMouseDragged, 320))
            c.view.layoutSubtreeIfNeeded(); XCTAssertEqual(c.thumbnailSidebarFrame.width, 320, accuracy: 0.5)
            if cancel { divider.keyDown(with: key(53)) }
            else { NotificationCenter.default.post(name: NSWindow.didResignKeyNotification, object: w) }
            c.view.layoutSubtreeIfNeeded(); XCTAssertEqual(c.thumbnailSidebarFrame.width, 176, accuracy: 0.5)
            divider.mouseUp(with: mouse(.leftMouseUp, 320))
            XCTAssertEqual(ReaderSidebarWidth(defaults: defaults).preferred, 176)
        }
    }
}
