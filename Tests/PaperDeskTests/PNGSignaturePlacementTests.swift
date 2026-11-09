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
        let bytes = try XCTUnwrap(bitmap.bitmapData); for i in 0..<(8 * 4 * 4) { bytes[i] = 0 }; bytes[0] = 255; bytes[3] = 255
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
            XCTAssertNotNil(reopened.page(at: 0)?.thumbnail(of: CGSize(width: 200, height: 200), for: .cropBox).tiffRepresentation)
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
}
