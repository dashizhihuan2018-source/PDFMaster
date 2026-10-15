import AppKit
import PaperCore
import XCTest
@testable import PaperDesk

private final class CanvasToolbarSentinelView: NSView {
    override func draw(_ dirtyRect: NSRect) {
        NSColor(srgbRed: 1, green: 0, blue: 1, alpha: 1).setFill(); bounds.fill()
    }
}

@MainActor final class CanvasClippingTests: XCTestCase {
    func testCanvasDoesNotPaintOverSiblingToolbarInHostedComposite() throws {
        let root = NSView(frame: CGRect(x: 0, y: 0, width: 800, height: 500))
        let toolbar = CanvasToolbarSentinelView(frame: CGRect(x: 0, y: 400, width: 800, height: 100))
        let canvas = CanvasView(session: CanvasSession())
        canvas.frame = CGRect(x: 0, y: 0, width: 800, height: 400)
        root.addSubview(toolbar)
        let window = NSWindow(contentRect: root.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = root
        defer { window.close() }
        root.layoutSubtreeIfNeeded()
        let baseline = try XCTUnwrap(root.bitmapImageRepForCachingDisplay(in: root.bounds))
        root.cacheDisplay(in: root.bounds, to: baseline)
        let expected = try XCTUnwrap(baseline.colorAt(x: baseline.pixelsWide / 2, y: baseline.pixelsHigh / 10)?.usingColorSpace(.sRGB))
        root.addSubview(canvas)
        let bitmap = try XCTUnwrap(root.bitmapImageRepForCachingDisplay(in: root.bounds))
        root.cacheDisplay(in: root.bounds, to: bitmap)
        let color = try XCTUnwrap(bitmap.colorAt(x: bitmap.pixelsWide / 2, y: bitmap.pixelsHigh / 10)?.usingColorSpace(.sRGB))
        XCTAssertEqual(color.redComponent, expected.redComponent, accuracy: 0.01)
        XCTAssertEqual(color.greenComponent, expected.greenComponent, accuracy: 0.01,
                       "Canvas background must not overpaint the sibling toolbar; use its baseline to account for color management")
        XCTAssertEqual(color.blueComponent, expected.blueComponent, accuracy: 0.01)
        XCTAssertTrue(canvas.clipsToBounds, "An infinite logical plane must still clip its native viewport on macOS 14+")
    }
    func testOffscreenBoardObjectDoesNotPaintOutsideViewport() throws {
        let outside = CanvasElement(kind: .rectangle, frame: CGRect(x: 100, y: -60, width: 300, height: 50), fillHex: "#00FF00")
        let view = CanvasView(session: CanvasSession(board: CanvasBoard(elements: [outside])))
        view.frame = CGRect(x: 0, y: 0, width: 600, height: 300)
        let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 600, pixelsHigh: 400,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
            bytesPerRow: 2400, bitsPerPixel: 32))
        let graphics = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: bitmap))
        let context = graphics.cgContext
        context.setFillColor(NSColor.magenta.cgColor); context.fill(CGRect(x: 0, y: 0, width: 600, height: 400))
        context.translateBy(x: 0, y: 300); context.scaleBy(x: 1, y: -1)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: true)
        defer { NSGraphicsContext.restoreGraphicsState() }
        view.draw(CGRect(x: 0, y: -100, width: 600, height: 400))
        let color = try XCTUnwrap(bitmap.colorAt(x: 200, y: 75)?.usingColorSpace(.sRGB))
        XCTAssertLessThan(color.greenComponent, 0.1, "Offscreen world geometry and oversized dirty regions must preserve neighboring chrome pixels")
        XCTAssertGreaterThan(color.redComponent, 0.9)
        XCTAssertGreaterThan(color.blueComponent, 0.9)
    }

    func testPartialRedrawPreservesUnaffectedViewportPixels() throws {
        let far = CanvasElement(kind:.rectangle,frame:CGRect(x:600,y:240,width:160,height:120),fillHex:"#0000FF")
        let view = CanvasView(session:CanvasSession(board:CanvasBoard(elements:[far])))
        view.frame = CGRect(x:0,y:0,width:800,height:600); view.showsDotGrid = false
        let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:800,pixelsHigh:600,
            bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:3200,bitsPerPixel:32))
        let context = try XCTUnwrap(NSGraphicsContext(bitmapImageRep:bitmap)?.cgContext)
        context.setFillColor(NSColor.magenta.cgColor); context.fill(view.bounds)
        context.translateBy(x:0,y:600); context.scaleBy(x:1,y:-1)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(cgContext:context,flipped:true)
        defer { NSGraphicsContext.restoreGraphicsState() }
        view.draw(CGRect(x:50,y:60,width:160,height:120))
        let unaffected = try XCTUnwrap(bitmap.colorAt(x:680,y:300)?.usingColorSpace(.sRGB))
        XCTAssertGreaterThan(unaffected.redComponent,0.9,"Drawing a local dirty region must not repaint unrelated objects elsewhere")
        XCTAssertLessThan(unaffected.greenComponent,0.1); XCTAssertGreaterThan(unaffected.blueComponent,0.9)
        let changed = try XCTUnwrap(bitmap.colorAt(x:100,y:100)?.usingColorSpace(.sRGB))
        XCTAssertGreaterThan(changed.greenComponent,0.9,"The requested dirty region still needs a background repaint")
    }
}
