import XCTest
import AppKit
import PaperCore
import PDFKit
@testable import PaperDesk

@MainActor final class PNGSignaturePlacementTests: XCTestCase {
    func testAspectFitClickAndDragOnRotatedNonzeroCrop() throws {
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 8, pixelsHigh: 4, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        let bytes = try XCTUnwrap(bitmap.bitmapData); for i in 0..<(8 * 4 * 4) { bytes[i] = 0 }; bytes[0] = 255; bytes[3] = 255
        let image = try SignatureImage(pngData: XCTUnwrap(bitmap.representation(using: .png, properties: [:])))
        let tool = PendingPlacement.imageSignature(image), crop = CGRect(x: 37, y: 61, width: 500, height: 700)
        for rotation in [0, 90, 180, 270] {
            let expected: CGFloat = rotation % 180 == 0 ? 2 : 0.5
            let click = try tool.bounds(from: CGPoint(x: 200, y: 300), to: CGPoint(x: 200, y: 300), within: crop, rotation: rotation)
            XCTAssertEqual(click.width / click.height, expected, accuracy: 0.001)
            XCTAssertEqual(rotation % 180 == 0 ? click.width : click.height, 160, accuracy: 0.001)
            let drag = try tool.bounds(from: CGPoint(x: 80, y: 120), to: CGPoint(x: 380, y: 320), within: crop, rotation: rotation)
            XCTAssertEqual(drag.width / drag.height, expected, accuracy: 0.001)
            XCTAssertEqual(drag.midX, 230, accuracy: 0.001); XCTAssertEqual(drag.midY, 220, accuracy: 0.001)
            XCTAssertTrue(crop.contains(drag))
        }
    }

    func testWorkbenchPlacementSaveReopenAndUndo() throws {
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 8, pixelsHigh: 4, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        let bytes = try XCTUnwrap(bitmap.bitmapData)
        for y in 0..<4 { for x in 0..<8 {
            let rgba: [UInt8] = y < 2 ? (x < 4 ? [0, 0, 0, 0] : [255, 0, 0, 255]) : (x < 4 ? [0, 0, 255, 255] : [0, 128, 0, 128])
            for i in 0..<4 { bytes[(y * 8 + x) * 4 + i] = rgba[i] }
        } }
        let image = try SignatureImage(pngData: XCTUnwrap(bitmap.representation(using: .png, properties: [:])))
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        for rotation in [0, 90, 180, 270] {
            let c = WorkbenchController(); _ = c.view; try c.workspace.createBlank()
            let source = try XCTUnwrap(c.workspace.document?.page(at: 0))
            source.setBounds(CGRect(x: 37, y: 61, width: 500, height: 700), for: .cropBox)
            if rotation > 0 { try c.workspace.rotate(page: 0, degrees: rotation) }
            c.refreshDocument(); let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
            c.beginPlacement(.imageSignature(image), accessibilityName: "图片签名")
            c.pdfView.beginGesture(on: page, at: CGPoint(x: 200, y: 300)); c.pdfView.finishGesture()
            XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
            let url = root.appendingPathComponent("\(rotation).pdf"); try c.workspace.save(to: url)
            let reopened = try XCTUnwrap(PDFDocument(url: url)); XCTAssertEqual(reopened.page(at: 0)?.annotations.first?.type, "Stamp")
            let thumbnail = try XCTUnwrap(reopened.page(at: 0)).thumbnail(of: CGSize(width: 700, height: 700), for: .cropBox)
            let rendered = try XCTUnwrap(NSBitmapImageRep(data: XCTUnwrap(thumbnail.tiffRepresentation)))
            var red: [(Int, Int)] = [], blue: [(Int, Int)] = []
            for y in 0..<rendered.pixelsHigh { for x in 0..<rendered.pixelsWide {
                guard let color = rendered.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { continue }
                if color.redComponent > 0.8 && color.greenComponent < 0.4 && color.blueComponent < 0.4 { red.append((x, y)) }
                if color.blueComponent > 0.8 && color.redComponent < 0.4 && color.greenComponent < 0.4 { blue.append((x, y)) }
            } }
            XCTAssertGreaterThan(red.count, 100, "rotation \(rotation)"); XCTAssertGreaterThan(blue.count, 100, "rotation \(rotation)")
            guard !red.isEmpty, !blue.isEmpty else { continue }
            func center(_ points: [(Int, Int)]) -> (Int, Int) { (points.map(\.0).reduce(0,+) / points.count, points.map(\.1).reduce(0,+) / points.count) }
            let r = center(red), b = center(blue)
            XCTAssertGreaterThan(r.0, b.0, "red must remain right of blue at rotation \(rotation)")
            XCTAssertLessThan(r.1, b.1, "red must remain above blue at rotation \(rotation)")
            let transparent = try XCTUnwrap(rendered.colorAt(x: b.0, y: r.1)?.usingColorSpace(.deviceRGB))
            XCTAssertGreaterThan(transparent.redComponent, 0.9); XCTAssertGreaterThan(transparent.greenComponent, 0.9); XCTAssertGreaterThan(transparent.blueComponent, 0.9)
            let half = try XCTUnwrap(rendered.colorAt(x: r.0, y: b.1)?.usingColorSpace(.deviceRGB))
            // PDFKit color management composites in a nonlinear color space.
            XCTAssertGreaterThan(half.redComponent, 0.3); XCTAssertLessThan(half.redComponent, 0.85)
            XCTAssertGreaterThan(half.greenComponent, 0.9)
            XCTAssertGreaterThan(half.blueComponent, 0.3); XCTAssertLessThan(half.blueComponent, 0.85)
            c.undo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
            let current = try XCTUnwrap(c.pdfView.document?.page(at: 0))
            c.beginPlacement(.imageSignature(image), accessibilityName: "图片签名")
            c.pdfView.beginGesture(on: current, at: CGPoint(x: 80, y: 120))
            c.pdfView.updateGesture(to: CGPoint(x: 380, y: 320)); c.pdfView.finishGesture()
            let bounds = try XCTUnwrap(c.workspace.document?.page(at: 0)?.annotations.first?.bounds)
            XCTAssertEqual(bounds.width / bounds.height, rotation % 180 == 0 ? 2 : 0.5, accuracy: 0.01)
            XCTAssertEqual(bounds.midX, 230, accuracy: 0.01); XCTAssertEqual(bounds.midY, 220, accuracy: 0.01)
            c.undo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        }
    }

    func testEscapeCancelsPendingImageGestureWithoutChangingDocument() throws {
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 8, pixelsHigh: 4, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        let image = try SignatureImage(pngData: XCTUnwrap(bitmap.representation(using: .png, properties: [:])))
        let c = WorkbenchController(); _ = c.view; try c.workspace.createBlank(); c.refreshDocument()
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0)), revision = c.workspace.revision
        c.beginPlacement(.imageSignature(image), accessibilityName: "图片签名")
        c.pdfView.beginGesture(on: page, at: CGPoint(x: 100, y: 200))
        c.pdfView.updateGesture(to: CGPoint(x: 300, y: 400))
        let escape = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
        c.pdfView.keyDown(with: escape); c.pdfView.finishGesture()
        XCTAssertFalse(c.pdfView.placementEnabled); XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
    }
}
