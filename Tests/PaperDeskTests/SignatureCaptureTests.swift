import AppKit
import XCTest
@testable import PaperDesk

@MainActor final class SignatureCaptureTests: XCTestCase {
    func testSignatureCanvasNeverMovesWindowOnMouseDown() {
        let canvas = SignatureCanvas(frame: NSRect(x: 0, y: 0, width: 420, height: 170))
        XCTAssertFalse(canvas.mouseDownCanMoveWindow)
        XCTAssertTrue(canvas.acceptsFirstResponder)
    }

    func testNativeDragCapturesSeparateNormalizedStrokesWithoutMovingWindow() throws {
        let canvas = SignatureCanvas(frame: NSRect(x: 0, y: 0, width: 420, height: 170))
        let window = NSWindow(contentRect: NSRect(x: 100, y: 100, width: 420, height: 170), styleMask: [.titled], backing: .buffered, defer: false)
        window.isMovableByWindowBackground = true; window.contentView = canvas
        let frame = window.frame
        func event(_ type: NSEvent.EventType, _ point: CGPoint) -> NSEvent {
            NSEvent.mouseEvent(with: type, location: canvas.convert(point, to: nil), modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
        }
        for y in [40.0, 100.0] {
            canvas.mouseDown(with: event(.leftMouseDown, CGPoint(x: 20, y: y)))
            canvas.mouseDragged(with: event(.leftMouseDragged, CGPoint(x: 180, y: y + 20)))
            canvas.mouseUp(with: event(.leftMouseUp, CGPoint(x: 300, y: y)))
        }
        XCTAssertEqual(window.frame, frame)
        XCTAssertEqual(canvas.strokes.count, 2)
        XCTAssertEqual(canvas.strokes[0].count, 3)
        XCTAssertGreaterThan(canvas.strokes[0].last!.x, canvas.strokes[0].first!.x)
        XCTAssertTrue(canvas.strokes.flatMap { $0 }.allSatisfy { (0...1).contains($0.x) && (0...1).contains($0.y) })
        window.contentView = nil
    }
}
