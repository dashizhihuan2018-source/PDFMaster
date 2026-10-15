import AppKit
import PaperCore
import XCTest
@testable import PaperDesk

/// Hidden native windows exercise hit testing and responder dispatch, not OS/CUA E2E.
@MainActor final class CanvasWindowDispatchTests: XCTestCase {
    private func host(_ controller: NSViewController) -> NSWindow {
        let window = NSWindow(contentRect: CGRect(x: 0, y: 0, width: 1280, height: 820),
                              styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = controller
        if let shell = controller as? ApplicationShellController { shell.installWindowChrome(in: window) }
        window.setContentSize(CGSize(width: 1280, height: 820))
        window.contentView?.layoutSubtreeIfNeeded()
        return window
    }
    private func mouse(_ type: NSEvent.EventType, at point: CGPoint, in view: NSView, clicks: Int = 1) -> NSEvent {
        NSEvent.mouseEvent(with: type, location: view.convert(point, to: nil), modifierFlags: [], timestamp: 0,
                           windowNumber: view.window!.windowNumber, context: nil, eventNumber: 0, clickCount: clicks, pressure: 1)!
    }
    private func dispatch(_ type: NSEvent.EventType, at point: CGPoint, in view: NSView, clicks: Int = 1) throws {
        let content = try XCTUnwrap(view.window?.contentView)
        let target = try XCTUnwrap(content.hitTest(view.convert(point, to: content)))
        let event = mouse(type, at: point, in: view, clicks: clicks)
        switch type {
        case .leftMouseDown: target.mouseDown(with: event)
        case .leftMouseDragged: target.mouseDragged(with: event)
        case .leftMouseUp: target.mouseUp(with: event)
        default: XCTFail("Unsupported fixture event")
        }
    }
    private func findButton(_ title: String, in view: NSView) -> NSButton? {
        if let button = view as? NSButton, button.title == title { return button }
        return view.subviews.lazy.compactMap { self.findButton(title, in: $0) }.first
    }
    func testHostedCanvasHitTestingAndHandlerDispatchCreateRectangle() throws {
        let controller = CanvasController()
        let window = host(controller); defer { window.close() }
        let canvas = controller.canvasView
        let content = try XCTUnwrap(window.contentView)
        let point = CGPoint(x: 300, y: 180)
        XCTAssertGreaterThan(canvas.bounds.width, 1100)
        XCTAssertGreaterThan(canvas.bounds.height, 600)
        XCTAssertTrue(content.hitTest(canvas.convert(point, to: content)) === canvas)
        try XCTUnwrap(findButton("矩形", in: controller.view)).performClick(nil)
        XCTAssertEqual(canvas.tool, .rectangle)
        XCTAssertTrue(window.firstResponder === canvas)
        try dispatch(.leftMouseDown, at: point, in: canvas)
        try dispatch(.leftMouseDragged, at: CGPoint(x: 420, y: 270), in: canvas)
        XCTAssertNotNil(canvas.previewElement)
        XCTAssertTrue(controller.session.board.elements.isEmpty)
        try dispatch(.leftMouseUp, at: CGPoint(x: 420, y: 270), in: canvas)
        XCTAssertEqual(controller.session.board.elements.count, 1)
        XCTAssertEqual(controller.session.board.elements.first?.frame, CGRect(x: 300, y: 180, width: 120, height: 90))
    }
    func testHostedCanvasDoubleClickStartsTextEditorAndFinishingCommits() throws {
        let item = CanvasElement(kind: .sticky, frame: CGRect(x: 100, y: 100, width: 200, height: 180), text: "原文字")
        let controller = CanvasController(session: CanvasSession(board: CanvasBoard(elements: [item])))
        let window = host(controller); defer { window.close() }
        let canvas = controller.canvasView
        let point = CGPoint(x: 140, y: 140)
        try dispatch(.leftMouseDown, at: point, in: canvas, clicks: 2)
        let editor = try XCTUnwrap(window.firstResponder as? NSTextView)
        editor.insertText("窗口输入", replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
        canvas.finishTextEditing()
        XCTAssertEqual(controller.session.board.elements.first?.text, "窗口输入")
        XCTAssertFalse(canvas.subviews.contains { $0 is NSTextField })
    }
    func testNewTextAndStickyImmediatelyAcceptKeyboardInput() throws {
        for tool: CanvasTool in [.text, .sticky] {
            let controller = CanvasController()
            let window = host(controller); defer { window.close() }
            let canvas = controller.canvasView
            canvas.tool = tool
            let point = CGPoint(x: 400, y: 300)
            canvas.mouseDown(with: mouse(.leftMouseDown, at: point, in: canvas))
            canvas.mouseUp(with: mouse(.leftMouseUp, at: point, in: canvas))
            XCTAssertEqual(controller.session.board.elements.count, 1)
            let editor = try XCTUnwrap(window.firstResponder as? NSTextView,
                                      "Creating \(tool.title) must focus its text editor, not silently discard typing")
            editor.insertText("直接输入", replacementRange: NSRange(location: 0, length: editor.string.utf16.count))
            canvas.finishTextEditing()
            XCTAssertEqual(controller.session.board.elements.first?.text, "直接输入")
            XCTAssertTrue(window.firstResponder === canvas)
        }
    }
    func testDoubleClickExistingTextWithCreationToolDoesNotCreateDuplicate() throws {
        for tool: CanvasTool in [.text, .sticky] {
            let item = CanvasElement(kind: tool.kind!, frame: CGRect(x: 300, y: 250, width: 240, height: 180), text: "已有文字")
            let controller = CanvasController(session: CanvasSession(board: CanvasBoard(elements: [item])))
            let window = host(controller); defer { window.close() }
            let canvas = controller.canvasView
            canvas.tool = tool
            let point = CGPoint(x: 340, y: 290)
            // Real double-click order includes the first down/up pair; testing
            // only an artificial clickCount=2 used to miss the duplicate.
            for clicks in [1, 2] {
                canvas.mouseDown(with: mouse(.leftMouseDown, at: point, in: canvas, clicks: clicks))
                canvas.mouseUp(with: mouse(.leftMouseUp, at: point, in: canvas, clicks: clicks))
            }
            XCTAssertEqual(controller.session.board.elements.map(\.id), [item.id])
            XCTAssertFalse(controller.session.canUndo, "Opening an editor must not insert a second object")
            XCTAssertTrue(window.firstResponder is NSTextView)
            canvas.finishTextEditing()
        }
    }
    func testShellHostedCanvasReceivesHitTestRoutedDragAfterToolSelection() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("canvas-dispatch-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: directory.appendingPathComponent("r.json")))
        let window = host(shell); defer { window.close() }
        let controller = shell.createCanvas().controller
        shell.view.layoutSubtreeIfNeeded()
        let canvas = controller.canvasView
        try XCTUnwrap(findButton("手绘", in: controller.view)).performClick(nil)
        try dispatch(.leftMouseDown, at: CGPoint(x: 100, y: 120), in: canvas)
        try dispatch(.leftMouseDragged, at: CGPoint(x: 180, y: 170), in: canvas)
        XCTAssertNotNil(canvas.previewElement)
        try dispatch(.leftMouseUp, at: CGPoint(x: 250, y: 130), in: canvas)
        XCTAssertEqual(controller.session.board.elements.first?.kind, .ink)
    }
    func testFinishingTextEditingRestoresCanvasKeyboardResponder() throws {
        let item = CanvasElement(kind: .text, frame: CGRect(x: 100, y: 100, width: 240, height: 70), text: "待编辑")
        let controller = CanvasController(session: CanvasSession(board: CanvasBoard(elements: [item])))
        let window = host(controller); defer { window.close() }
        controller.canvasView.activate(item.id)
        XCTAssertTrue(window.firstResponder is NSTextView)
        controller.canvasView.finishTextEditing()
        XCTAssertTrue(window.firstResponder === controller.canvasView,
                      "A dismissed field editor must not retain Delete/Space/undo keyboard focus")
        let delete = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
            windowNumber: window.windowNumber, context: nil, characters: "\u{7f}", charactersIgnoringModifiers: "\u{7f}",
            isARepeat: false, keyCode: 51))
        window.sendEvent(delete)
        XCTAssertTrue(controller.session.board.elements.isEmpty, "Delete must reach the selected canvas object, not its dismissed text editor")
        controller.session.undo()
        XCTAssertEqual(controller.session.board.elements.first?.id, item.id)
    }
    func testShellActivationMakesCanvasFirstResponderInsteadOfPreviousPDF() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("canvas-activation-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: directory.appendingPathComponent("r.json")))
        let window = host(shell); defer { window.close() }
        let pdf = shell.createBlankDocument()
        XCTAssertTrue(window.makeFirstResponder(pdf.controller.pdfView))
        let canvas = shell.createCanvas()
        shell.view.layoutSubtreeIfNeeded()
        XCTAssertTrue(window.firstResponder === canvas.controller.canvasView,
                      "Creating a canvas must hand keyboard commands to the visible board")
        shell.activate(tabID: pdf.id)
        XCTAssertTrue(window.makeFirstResponder(pdf.controller.pdfView))
        shell.activate(tabID: canvas.id)
        XCTAssertTrue(window.firstResponder === canvas.controller.canvasView,
                      "Returning to a canvas tab must not leave the hidden PDF as keyboard responder")
    }
    func testEveryCanvasToolOptsOutOfWindowBackgroundDragging() {
        let controller = CanvasController()
        let window = host(controller); defer { window.close() }
        // Match the shipping full-size-content window. This is the decisive
        // NSWindow background-drag eligibility check, not OS mouse-event E2E.
        window.isMovableByWindowBackground = true
        let canvas = controller.canvasView
        for tool in CanvasTool.allCases {
            canvas.tool = tool
            XCTAssertFalse(canvas.mouseDownCanMoveWindow,
                           "\(tool.title) must never enter native window-background dragging")
        }
        XCTAssertTrue(window.isMovableByWindowBackground, "Canvas must not disable the application's custom title-bar dragging")
    }
    func testBackgroundMovableWindowHitTestRoutesCanvasContentGesturesWithoutMovingFrame() throws {
        let controller = CanvasController()
        let window = host(controller); defer { window.close() }
        window.isMovableByWindowBackground = true
        let canvas = controller.canvasView
        let initialWindowFrame = window.frame
        // Hidden native-window hit testing + handlers only. The locked host
        // cannot acquire a key window for NSWindow.sendEvent/OS input E2E.
        canvas.tool = .rectangle
        try dispatch(.leftMouseDown, at: CGPoint(x: 300, y: 180), in: canvas)
        try dispatch(.leftMouseDragged, at: CGPoint(x: 420, y: 270), in: canvas)
        XCTAssertNotNil(canvas.previewElement)
        XCTAssertTrue(controller.session.board.elements.isEmpty)
        try dispatch(.leftMouseUp, at: CGPoint(x: 420, y: 270), in: canvas)
        XCTAssertEqual(controller.session.board.elements.first?.frame, CGRect(x: 300, y: 180, width: 120, height: 90))
        canvas.tool = .selection
        try dispatch(.leftMouseDown, at: CGPoint(x: 350, y: 220), in: canvas)
        try dispatch(.leftMouseDragged, at: CGPoint(x: 430, y: 260), in: canvas)
        try dispatch(.leftMouseUp, at: CGPoint(x: 430, y: 260), in: canvas)
        XCTAssertEqual(controller.session.board.elements.first?.frame, CGRect(x: 380, y: 220, width: 120, height: 90))
        canvas.tool = .hand
        try dispatch(.leftMouseDown, at: CGPoint(x: 200, y: 180), in: canvas)
        try dispatch(.leftMouseDragged, at: CGPoint(x: 280, y: 220), in: canvas)
        try dispatch(.leftMouseUp, at: CGPoint(x: 280, y: 220), in: canvas)
        XCTAssertEqual(canvas.viewport.offset, CGPoint(x: 80, y: 40))
        XCTAssertEqual(controller.session.board.elements.count, 1)
        XCTAssertEqual(window.frame, initialWindowFrame)
    }
    func testPointerJitterDoesNotCreateTinyCanvasObjectsAtAnyZoom() {
        for scale: CGFloat in [0.05, 0.5, 1, 8] {
            for tool: CanvasTool in [.rectangle, .ellipse, .connector, .ink] {
                let controller = CanvasController()
                let canvas = controller.canvasView
                canvas.tool = tool; canvas.zoom(to: scale, anchor: .zero)
                canvas.beginGesture(at: CGPoint(x: 100, y: 120))
                canvas.updateGesture(to: CGPoint(x: 101, y: 121))
                canvas.finishGesture()
                XCTAssertTrue(controller.session.board.elements.isEmpty, "Click jitter must not create \(tool.title) at \(scale)x")
                XCTAssertFalse(controller.session.canUndo)
            }
        }
    }
}
