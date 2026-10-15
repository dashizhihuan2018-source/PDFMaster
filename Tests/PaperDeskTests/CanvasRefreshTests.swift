import AppKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor final class CanvasRefreshTests: XCTestCase {
    private func controller(_ board: CanvasBoard = CanvasBoard()) -> CanvasController {
        let c = CanvasController(session: CanvasSession(board: board))
        let window = NSWindow(contentRect: CGRect(x: 0, y: 0, width: 900, height: 650),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentViewController = c
        window.setContentSize(CGSize(width:900,height:650))
        c.view.frame = CGRect(x:0,y:0,width:900,height:650)
        c.view.layoutSubtreeIfNeeded()
        addTeardownBlock { MainActor.assumeIsolated { window.close() } }
        c.canvasView.showsDotGrid = false
        return c
    }
    private func imageData(_ color: NSColor) throws -> Data {
        let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 40, pixelsHigh: 40,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 160, bitsPerPixel: 32))
        let context = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: bitmap)?.cgContext)
        context.setFillColor(color.cgColor); context.fill(CGRect(x: 0, y: 0, width: 40, height: 40))
        return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
    }
    // Inspect the real private cache without exposing test-only API on CanvasView.
    private func cachedImage(_ id: UUID, in view: CanvasView) -> NSImage? {
        let cache = Mirror(reflecting: view).children.first { $0.label == "imageCache" }?.value as? [UUID: NSImage]
        return cache?[id]
    }
    func testTitleAndSavedStateChangesDoNotInvalidateWorkSurface() throws {
        let c = controller(); c.canvasView.needsDisplay = true
        XCTAssertTrue(c.canvasView.needsDisplay,"Fixture must retain a real dirty region before testing invalidation")
        // Isolate the work surface: updating floating labels legitimately
        // repaints the small region behind translucent chrome in AppKit.
        let v = c.canvasView; c.session.onChange = { [weak v] in v?.refresh() }
        c.view.display()
        c.canvasView.needsDisplay = false
        XCTAssertFalse(c.canvasView.needsDisplay)
        try c.session.mutate { $0.title = "Only chrome changes" }
        XCTAssertFalse(c.canvasView.needsDisplay, "Metadata does not change a single canvas pixel")
        c.canvasView.needsDisplay = false; c.session.markSaved()
        XCTAssertFalse(c.canvasView.needsDisplay)
        c.session.undo(); XCTAssertFalse(c.canvasView.needsDisplay)
    }
    func testUnrelatedShapeUndoRetainsDecodedImageIdentity() throws {
        let image = CanvasElement(kind: .image, frame: CGRect(x: 400, y: 180, width: 200, height: 200),
            assetData: try imageData(.red), assetName: "fixture.png")
        let c = controller(CanvasBoard(elements: [image])), v = c.canvasView
        _ = try XCTUnwrap(v.renderPNGData())
        let decoded = try XCTUnwrap(cachedImage(image.id, in: v))
        try c.session.mutate { $0.elements.append(CanvasElement(kind: .rectangle, frame: CGRect(x: 100, y: 160, width: 100, height: 80))) }
        XCTAssertTrue(cachedImage(image.id, in: v) === decoded)
        c.session.undo()
        XCTAssertTrue(cachedImage(image.id, in: v) === decoded, "Undoing vectors must not re-decode every PDF/image thumbnail")
    }
    func testOffscreenMutationDoesNotInvalidateVisibleViewport() throws {
        let c = controller(); c.canvasView.needsDisplay = true
        XCTAssertTrue(c.canvasView.needsDisplay,"Fixture must retain a real dirty region before testing invalidation")
        let v = c.canvasView; c.session.onChange = { [weak v] in v?.refresh() }
        c.view.display()
        c.canvasView.needsDisplay = false
        XCTAssertFalse(c.canvasView.needsDisplay)
        try c.session.mutate { $0.elements.append(CanvasElement(kind: .rectangle,
            frame: CGRect(x: 8000, y: 6000, width: 100, height: 80))) }
        XCTAssertFalse(c.canvasView.needsDisplay)
        c.session.undo(); XCTAssertFalse(c.canvasView.needsDisplay)
    }
    func testChangedAssetWithSameIDRendersNewContentAndUndoRestoresOldContent() throws {
        let item = CanvasElement(kind: .image, frame: CGRect(x: 300, y: 180, width: 200, height: 200),
            assetData: try imageData(.red), assetName: "fixture.png")
        let c = controller(CanvasBoard(elements: [item])), v = c.canvasView
        func pixel() throws -> NSColor {
            let rep = try XCTUnwrap(NSBitmapImageRep(data: try XCTUnwrap(v.renderPNGData())))
            return try XCTUnwrap(rep.colorAt(x: 400, y: 280)?.usingColorSpace(.sRGB))
        }
        XCTAssertGreaterThan(try pixel().redComponent, 0.8)
        let blue = try imageData(.blue)
        try c.session.mutate { $0.elements[0].assetData = blue }
        XCTAssertGreaterThan(try pixel().blueComponent, 0.8)
        XCTAssertLessThan(try pixel().redComponent, 0.2)
        c.session.undo(); XCTAssertGreaterThan(try pixel().redComponent, 0.8)
    }

    func testOffscreenAttachmentIsDecodedOnlyWhenItEntersTheViewport() throws {
        let item = CanvasElement(kind:.image,frame:CGRect(x:3000,y:200,width:200,height:200),
            assetData:try imageData(.blue),assetName:"offscreen.png")
        let c = controller(CanvasBoard(elements:[item])), v = c.canvasView
        _ = try XCTUnwrap(v.renderPNGData())
        XCTAssertNil(cachedImage(item.id,in:v),"Invisible cards must not decode thumbnails during local vector redraws")
        v.pan(by:CGSize(width:-2700,height:0)); _ = try XCTUnwrap(v.renderPNGData())
        XCTAssertNotNil(cachedImage(item.id,in:v))
    }
}
