import AppKit
import PaperCore
import XCTest
@testable import PaperDesk

/// AppKit exposes no public magnify-event factory; only the event payload is
/// supplied here. The actual CanvasView responder/viewport code is exercised.
private final class CanvasMagnificationEvent: NSEvent {
    var amount: CGFloat = 0
    var point = CGPoint(x:240,y:170)
    override var type: NSEvent.EventType { .magnify }
    override var magnification: CGFloat { amount }
    override var locationInWindow: NSPoint { point }
}

@MainActor final class CanvasNavigationThemeTests: XCTestCase {
    private func scroll(_ units:CGScrollEventUnit, delta:Int32, flags:CGEventFlags = []) throws -> NSEvent {
        let cg = try XCTUnwrap(CGEvent(scrollWheelEvent2Source:nil,units:units,wheelCount:1,wheel1:delta,wheel2:0,wheel3:0))
        cg.flags = flags; cg.location = CGPoint(x:240,y:170)
        return try XCTUnwrap(NSEvent(cgEvent:cg))
    }
    private func button(_ title:String,in view:NSView) -> NSButton? {
        if let button = view as? NSButton, button.title == title { return button }
        return view.subviews.lazy.compactMap { self.button(title,in:$0) }.first
    }
    func testDiscreteWheelZoomsWithoutCommandAroundPointer() throws {
        let c = CanvasController(); c.view.frame = CGRect(x:0,y:0,width:900,height:650); c.view.layoutSubtreeIfNeeded()
        let v = c.canvasView, event = try scroll(.line,delta:3)
        XCTAssertFalse(event.hasPreciseScrollingDeltas)
        let anchor = v.convert(event.locationInWindow,from:nil), world = v.viewport.worldPoint(from:anchor)
        v.scrollWheel(with:event)
        XCTAssertGreaterThan(v.viewport.scale,1)
        XCTAssertEqual(v.viewport.worldPoint(from:anchor).x,world.x,accuracy:0.0001)
        XCTAssertEqual(v.viewport.worldPoint(from:anchor).y,world.y,accuracy:0.0001)
        XCTAssertFalse(c.session.isDirty); XCTAssertFalse(c.session.canUndo)
    }
    func testPreciseTrackpadScrollZoomsWithoutCommandAndShiftPans() throws {
        let v = CanvasView(session:CanvasSession()); v.frame = CGRect(x:0,y:0,width:900,height:650)
        let event = try scroll(.pixel,delta:20)
        XCTAssertTrue(event.hasPreciseScrollingDeltas)
        let anchor = v.convert(event.locationInWindow,from:nil), world = v.viewport.worldPoint(from:anchor)
        v.scrollWheel(with:event); XCTAssertGreaterThan(v.viewport.scale,1)
        XCTAssertEqual(v.viewport.worldPoint(from:anchor).x,world.x,accuracy:0.0001)
        let scale = v.viewport.scale, offset = v.viewport.offset
        v.scrollWheel(with:try scroll(.pixel,delta:20,flags:.maskShift))
        XCTAssertEqual(v.viewport.scale,scale); XCTAssertNotEqual(v.viewport.offset,offset)
        v.tool = .hand; let before = v.viewport.offset
        v.scrollWheel(with:event); XCTAssertEqual(v.viewport.scale,scale); XCTAssertNotEqual(v.viewport.offset,before)
    }
    func testBlackWhiteTogglePersistsAndChangesOnlyCanvasChromeAppearance() throws {
        let key = "PDFMaster.canvasBackground", suite = "CanvasThemeTest-\(UUID())"
        let defaults = try XCTUnwrap(UserDefaults(suiteName:suite))
        defer { defaults.removePersistentDomain(forName:suite) }
        defaults.set("light",forKey:key)
        let c = CanvasController(preferences:defaults); _ = c.view
        let global = NSApp?.appearance
        let toggle = try XCTUnwrap(button("切换黑白背景",in:c.view))
        XCTAssertEqual(c.view.appearance?.name,.aqua)
        toggle.performClick(nil)
        XCTAssertEqual(defaults.string(forKey:key),"dark")
        XCTAssertEqual(c.view.appearance?.name,.darkAqua)
        XCTAssertEqual(c.canvasView.effectiveAppearance.bestMatch(from:[.aqua,.darkAqua]),.darkAqua)
        for capsule in c.view.subviews.compactMap({ $0 as? CanvasGlassCapsule }) {
            XCTAssertEqual(capsule.effectiveAppearance.bestMatch(from:[.aqua,.darkAqua]),.darkAqua)
        }
        let reopened = CanvasController(preferences:defaults); _ = reopened.view
        XCTAssertEqual(reopened.view.appearance?.name,.darkAqua)
        try XCTUnwrap(button("切换黑白背景",in:reopened.view)).performClick(nil)
        XCTAssertEqual(defaults.string(forKey:key),"light"); XCTAssertEqual(reopened.view.appearance?.name,.aqua)
        XCTAssertTrue(NSApp?.appearance === global)
        XCTAssertFalse(c.session.isDirty); XCTAssertFalse(c.session.canUndo)
    }
    func testAIEntryIsVisibleAndUnavailableUntilRealServiceIsAttached() throws {
        let c = CanvasController(); _ = c.view
        let entry = try XCTUnwrap(button("AI创作",in:c.view))
        XCTAssertFalse(entry.isHidden); XCTAssertFalse(entry.isEnabled)
        XCTAssertEqual(entry.toolTip,"AI创作（暂不可用）")
        XCTAssertEqual(entry.accessibilityLabel(),"AI创作")
        var activations = 0; c.onShowAI = { activations += 1 }
        XCTAssertTrue(entry.isEnabled); entry.performClick(nil); XCTAssertEqual(activations,1)
        XCTAssertEqual(entry.toolTip,"AI创作")
        c.onShowAI = nil; XCTAssertFalse(entry.isEnabled)
        XCTAssertEqual(entry.toolTip,"AI创作（暂不可用）")
        entry.performClick(nil); XCTAssertEqual(activations,1)
        XCTAssertFalse(c.session.isDirty)
    }
    func testExtremeWheelZoomClampsAndRetainsAnchorWithoutUndo() throws {
        let s = CanvasSession(), v = CanvasView(session:s); v.frame = CGRect(x:0,y:0,width:900,height:650)
        let up = try scroll(.line,delta:10000), anchor = v.convert(up.locationInWindow,from:nil)
        let initial = v.viewport.worldPoint(from:anchor)
        for _ in 0..<4 { v.scrollWheel(with:up) }
        XCTAssertEqual(v.viewport.scale,8)
        XCTAssertEqual(v.viewport.worldPoint(from:anchor).x,initial.x,accuracy:0.0001)
        let down = try scroll(.line,delta:-10000)
        for _ in 0..<4 { v.scrollWheel(with:down) }
        XCTAssertEqual(v.viewport.scale,0.05)
        XCTAssertEqual(v.viewport.worldPoint(from:anchor).y,initial.y,accuracy:0.0001)
        XCTAssertFalse(s.canUndo); XCTAssertFalse(s.isDirty)
    }
    func testNativeMagnifyHandlerAnchorsPinchAndClampsInvalidScale() {
        let s = CanvasSession(), v = CanvasView(session:s); v.frame = CGRect(x:0,y:0,width:900,height:650)
        let event = CanvasMagnificationEvent(); event.amount = 0.5
        let anchor = v.convert(event.locationInWindow,from:nil), initial = v.viewport.worldPoint(from:anchor)
        v.magnify(with:event)
        XCTAssertEqual(v.viewport.scale,1.5)
        XCTAssertEqual(v.viewport.worldPoint(from:anchor).x,initial.x,accuracy:0.0001)
        XCTAssertEqual(v.viewport.worldPoint(from:anchor).y,initial.y,accuracy:0.0001)
        event.amount = 100; v.magnify(with:event); XCTAssertEqual(v.viewport.scale,8)
        event.amount = -2; v.magnify(with:event); XCTAssertEqual(v.viewport.scale,0.05)
        let before = v.viewport
        event.amount = .nan; v.magnify(with:event); XCTAssertEqual(v.viewport,before)
        XCTAssertFalse(s.canUndo); XCTAssertFalse(s.isDirty)
    }
    func testThemeActuallyRendersWhiteAndBlackBackgroundPixels() throws {
        let suite = "CanvasThemeTest-\(UUID())", defaults = try XCTUnwrap(UserDefaults(suiteName:suite))
        defer { defaults.removePersistentDomain(forName:suite) }
        let c = CanvasController(preferences:defaults); c.view.frame = CGRect(x:0,y:0,width:800,height:600); c.view.layoutSubtreeIfNeeded()
        c.canvasView.showsDotGrid = false
        let white = try XCTUnwrap(NSBitmapImageRep(data:try XCTUnwrap(c.canvasView.renderPNGData()))).colorAt(x:400,y:300)!
        XCTAssertGreaterThan(white.usingColorSpace(.sRGB)!.redComponent,0.95)
        try XCTUnwrap(button("切换黑白背景",in:c.view)).performClick(nil)
        let black = try XCTUnwrap(NSBitmapImageRep(data:try XCTUnwrap(c.canvasView.renderPNGData()))).colorAt(x:400,y:300)!
        XCTAssertLessThan(black.usingColorSpace(.sRGB)!.redComponent,0.05)
        XCTAssertFalse(c.session.isDirty)
    }
    func testSpaceScrollPanKeepsZoomScaleAndZeroWheelDoesNothing() throws {
        let v = CanvasView(session:CanvasSession()); v.frame = CGRect(x:0,y:0,width:900,height:650)
        let key = try XCTUnwrap(NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:[],timestamp:0,windowNumber:0,
            context:nil,characters:" ",charactersIgnoringModifiers:" ",isARepeat:false,keyCode:49))
        v.keyDown(with:key); v.scrollWheel(with:try scroll(.pixel,delta:20))
        XCTAssertEqual(v.viewport.scale,1); XCTAssertNotEqual(v.viewport.offset,.zero)
        v.keyUp(with:key); let before = v.viewport
        v.scrollWheel(with:try scroll(.line,delta:0)); XCTAssertEqual(v.viewport,before)
    }

    func testWheelZoomDiscardsUncommittedStrokeBeforeCoordinateSystemChanges() throws {
        let s = CanvasSession(), v = CanvasView(session:s)
        v.frame = CGRect(x:0,y:0,width:900,height:650); v.tool = .ink
        v.beginGesture(at:CGPoint(x:100,y:100)); v.updateGesture(to:CGPoint(x:160,y:150))
        XCTAssertNotNil(v.previewElement)
        v.scrollWheel(with:try scroll(.line,delta:3))
        XCTAssertNil(v.previewElement, "A draft in old world coordinates must not survive viewport navigation")
        v.updateGesture(to:CGPoint(x:220,y:160)); v.finishGesture()
        XCTAssertTrue(s.board.elements.isEmpty); XCTAssertFalse(s.canUndo)
        XCTAssertGreaterThan(v.viewport.scale,1)
    }

    func testScrollPanDiscardsUncommittedMoveWithoutMovingTheObject() throws {
        let item = CanvasElement(kind:.rectangle,frame:CGRect(x:80,y:80,width:160,height:90))
        let s = CanvasSession(board:CanvasBoard(elements:[item])), v = CanvasView(session:s)
        v.frame = CGRect(x:0,y:0,width:900,height:650)
        v.beginGesture(at:CGPoint(x:100,y:100)); v.updateGesture(to:CGPoint(x:130,y:120))
        XCTAssertNotNil(v.previewElement)
        v.scrollWheel(with:try scroll(.pixel,delta:20,flags:.maskShift))
        XCTAssertNil(v.previewElement)
        v.updateGesture(to:CGPoint(x:180,y:140)); v.finishGesture()
        XCTAssertEqual(s.board.elements,[item]); XCTAssertFalse(s.canUndo)
        XCTAssertNotEqual(v.viewport.offset,.zero)
    }

    func testLosingKeyboardFocusReleasesTemporarySpacePanMode() throws {
        let c = CanvasController(), window = NSWindow(contentRect:CGRect(x:0,y:0,width:900,height:650),
            styleMask:[.titled],backing:.buffered,defer:false)
        window.isReleasedWhenClosed = false; window.contentViewController = c
        defer { window.close() }; c.view.layoutSubtreeIfNeeded()
        let v = c.canvasView
        XCTAssertTrue(window.makeFirstResponder(v))
        let key = try XCTUnwrap(NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:[],timestamp:0,windowNumber:window.windowNumber,
            context:nil,characters:" ",charactersIgnoringModifiers:" ",isARepeat:false,keyCode:49))
        v.keyDown(with:key)
        XCTAssertTrue(window.makeFirstResponder(nil)); XCTAssertTrue(window.makeFirstResponder(v))
        v.scrollWheel(with:try scroll(.pixel,delta:20))
        XCTAssertGreaterThan(v.viewport.scale,1,"A missing keyUp while another control owns focus must not leave the canvas in hand mode")
    }

    func testLosingKeyboardFocusDiscardsDraftWithoutUndoEntry() throws {
        let c = CanvasController(), window = NSWindow(contentRect:CGRect(x:0,y:0,width:900,height:650),
            styleMask:[.titled],backing:.buffered,defer:false)
        window.isReleasedWhenClosed = false; window.contentViewController = c
        defer { window.close() }; c.view.layoutSubtreeIfNeeded()
        let v = c.canvasView; XCTAssertTrue(window.makeFirstResponder(v))
        v.tool = .rectangle; v.beginGesture(at:CGPoint(x:100,y:100)); v.updateGesture(to:CGPoint(x:200,y:160))
        XCTAssertNotNil(v.previewElement)
        XCTAssertTrue(window.makeFirstResponder(nil))
        XCTAssertNil(v.previewElement)
        v.finishGesture(); XCTAssertTrue(c.session.board.elements.isEmpty); XCTAssertFalse(c.session.canUndo)
    }

    func testOwnWindowDeactivationDropsDraftAndSpaceWithoutStealingFocus() throws {
        let c = CanvasController(), window = NSWindow(contentRect:CGRect(x:0,y:0,width:900,height:650),
            styleMask:[.titled],backing:.buffered,defer:false)
        window.isReleasedWhenClosed = false; window.contentViewController = c
        defer { window.close() }; c.view.layoutSubtreeIfNeeded()
        let v = c.canvasView; XCTAssertTrue(window.makeFirstResponder(v))
        v.tool = .ink; v.beginGesture(at:CGPoint(x:100,y:100)); v.updateGesture(to:CGPoint(x:170,y:160))
        let key = try XCTUnwrap(NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:[],timestamp:0,windowNumber:window.windowNumber,
            context:nil,characters:" ",charactersIgnoringModifiers:" ",isARepeat:false,keyCode:49))
        v.keyDown(with:key)
        // A native key-window notification payload, not physical trackpad/CUA.
        NotificationCenter.default.post(name:NSWindow.didResignKeyNotification,object:window)
        XCTAssertNil(v.previewElement); XCTAssertTrue(window.firstResponder === v)
        v.finishGesture(); XCTAssertFalse(c.session.canUndo)
        v.scrollWheel(with:try scroll(.pixel,delta:20)); XCTAssertGreaterThan(v.viewport.scale,1)
    }

    func testOtherWindowDeactivationDoesNotDiscardThisCanvasDraft() {
        let c = CanvasController(), window = NSWindow(contentRect:CGRect(x:0,y:0,width:900,height:650),
            styleMask:[.titled],backing:.buffered,defer:false)
        window.isReleasedWhenClosed = false; window.contentViewController = c
        let other = NSWindow(contentRect:CGRect(x:0,y:0,width:200,height:150),styleMask:[.titled],backing:.buffered,defer:false)
        other.isReleasedWhenClosed = false
        defer { other.close(); window.close() }
        let v = c.canvasView; v.tool = .rectangle
        v.beginGesture(at:CGPoint(x:100,y:100)); v.updateGesture(to:CGPoint(x:200,y:160))
        NotificationCenter.default.post(name:NSWindow.didResignKeyNotification,object:other)
        XCTAssertNotNil(v.previewElement)
        v.finishGesture(); XCTAssertEqual(c.session.board.elements.count,1)
    }
}
