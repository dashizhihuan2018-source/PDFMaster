import XCTest
import AppKit
import PDFKit
import PaperCore
@testable import PaperDesk

@MainActor final class InkRenderingSharpnessTests: XCTestCase {
    private func recordEvidence(_ data: Data, name: String) throws {
        guard let directory = ProcessInfo.processInfo.environment["PDFMASTER_INK_EVIDENCE_DIR"] else { return }
        let url = URL(fileURLWithPath: directory, isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        try data.write(to: url.appendingPathComponent(name), options: .atomic)
    }

    func testDrawingNextStrokeDoesNotRegenerateExistingCommittedVectorPaths() throws {
        let (view, page, _) = fixture()
        let overlay = try XCTUnwrap(view.subviews.first { $0.layer?.zPosition == 1000 })
        let layer = try XCTUnwrap(overlay.layer?.sublayers?.last?.sublayers?.first as? CAShapeLayer)
        let before = try XCTUnwrap(layer.path)
        let mask = try XCTUnwrap(layer.mask)
        view.inkEnabled = true
        view.beginGesture(on: page, at: CGPoint(x: 125, y: 130))
        for i in 0..<20 { view.updateGesture(to: CGPoint(x: 140 + i, y: 140 + i)) }
        XCTAssertTrue(layer.path === before,
                      "Mouse samples must not repeatedly rebuild every already committed ink path")
        XCTAssertTrue(layer.mask === mask, "Mouse samples must not allocate new clipping layers for unchanged committed ink")
    }

    private func fixture(rotation: Int = 0, scale: CGFloat = 4.7) -> (InkPDFView, PDFPage, PDFAnnotation) {
        let view = InkPDFView(frame: CGRect(x: 0, y: 0, width: 900, height: 700))
        let document = PDFDocument(), page = PDFPage()
        page.setBounds(CGRect(x: 0, y: 0, width: 300, height: 300), for: .mediaBox)
        page.rotation = rotation
        let annotation = PDFAnnotation(bounds: CGRect(x: 110, y: 120, width: 100, height: 100), forType: .ink, withProperties: nil)
        // PDF InkList stores polylines, not Bezier control points. Use exactly
        // the adaptive polyline used by PDF Master's brush and PDF writer.
        let path = InkStrokePath.bezier(InkStrokePath.make(points: [CGPoint(x: 10, y: 20), CGPoint(x: 30, y: 90), CGPoint(x: 60, y: 5), CGPoint(x: 80, y: 70)]))
        annotation.add(path); annotation.color = .blue
        let border = PDFBorder(); border.lineWidth = 2; annotation.border = border
        page.addAnnotation(annotation); document.insert(page, at: 0)
        view.document = document; view.autoScales = false; view.scaleFactor = scale
        view.layoutDocumentView(); view.go(to: annotation.bounds, on: page); view.layout()
        return (view, page, annotation)
    }

    func testErasingCommittedStrokeClearsItsVectorPixelsWithoutReplacingDocument() throws {
        let (view, page, annotation) = fixture()
        let document = view.document
        let painted = try XCTUnwrap(view.renderShapeOverlay())
        page.removeAnnotation(annotation); view.refreshCommittedInkRendering()
        XCTAssertNotEqual(painted, try XCTUnwrap(view.renderShapeOverlay()))
        XCTAssertTrue(view.document === document)
        XCTAssertTrue(annotation.shouldDisplay, "Removed presentation entries restore their original native display flag")
        page.addAnnotation(annotation); view.refreshCommittedInkRendering()
        XCTAssertEqual(painted, try XCTUnwrap(view.renderShapeOverlay()))
    }

    func testSavedPresentationKeepsVisibleEditableNativeInkAndHiddenSourceInkStaysHidden() throws {
        let (view, page, annotation) = fixture()
        XCTAssertFalse(annotation.shouldDisplay, "Only PDFKit's cached screen rendering is suppressed")
        let hidden = PDFAnnotation(bounds: CGRect(x: 10, y: 10, width: 20, height: 20), forType: .ink, withProperties: nil)
        let hiddenPath = NSBezierPath(); hiddenPath.move(to: .zero); hiddenPath.line(to: CGPoint(x: 10, y: 10))
        hidden.add(hiddenPath); hidden.shouldDisplay = false; page.addAnnotation(hidden)
        view.refreshCommittedInkRendering()
        let saved = try XCTUnwrap(view.presentationDataRepresentation())
        let reopened = try XCTUnwrap(PDFDocument(data: saved))
        let savedInk = try XCTUnwrap(reopened.page(at: 0)?.annotations.first)
        XCTAssertTrue(savedInk.shouldDisplay)
        XCTAssertEqual(savedInk.type, "Ink")
        XCTAssertEqual(savedInk.paths?.first?.elementCount, annotation.paths?.first?.elementCount)
        XCTAssertFalse(try XCTUnwrap(reopened.page(at: 0)?.annotations.last).shouldDisplay)
        XCTAssertFalse(annotation.shouldDisplay, "Serialization must not permanently change screen suppression")
        view.document = nil
        XCTAssertTrue(annotation.shouldDisplay)
        XCTAssertFalse(hidden.shouldDisplay)
    }

    func testSavedReopenedInkRendersSmoothAntialiasedPixelsAtHighZoomForEveryRotation() throws {
        for rotation in [0, 90, 180, 270] {
            let (view, _, _) = fixture(rotation: rotation, scale: 6.4)
            let original = try XCTUnwrap(view.renderShapeOverlay())
            let saved = try XCTUnwrap(view.presentationDataRepresentation())
            view.document = try XCTUnwrap(PDFDocument(data: saved))
            view.layoutDocumentView()
            let page = try XCTUnwrap(view.document?.page(at: 0))
            let annotation = try XCTUnwrap(page.annotations.first)
            view.go(to: annotation.bounds, on: page)
            view.refreshCommittedInkRendering()
            let reopened = try XCTUnwrap(view.renderShapeOverlay())
            try recordEvidence(original, name: "ink-before-save-640-rotation\(rotation).png")
            try recordEvidence(reopened, name: "ink-reopened-640-rotation\(rotation).png")
            let bitmap = try XCTUnwrap(NSBitmapImageRep(data: reopened))
            var opaque = 0, antialiased = 0
            for y in stride(from: 0, to: bitmap.pixelsHigh, by: 2) {
                for x in stride(from: 0, to: bitmap.pixelsWide, by: 2) {
                    let alpha = bitmap.colorAt(x: x, y: y)?.alphaComponent ?? 0
                    if alpha > 0.98 { opaque += 1 }
                    else if alpha > 0.01 { antialiased += 1 }
                }
            }
            XCTAssertGreaterThan(opaque, 100, "The actual committed stroke must be painted at \(rotation)°")
            XCTAssertGreaterThan(antialiased, 20, "Curved vector edges should remain antialiased rather than block-scaled at \(rotation)°")
            let originalBitmap = try XCTUnwrap(NSBitmapImageRep(data: original))
            var alphaMass: CGFloat = 0, deltaMass: CGFloat = 0, largestDelta: CGFloat = 0
            for y in 0..<bitmap.pixelsHigh {
                for x in 0..<bitmap.pixelsWide {
                    let a = originalBitmap.colorAt(x: x, y: y)?.alphaComponent ?? 0
                    let b = bitmap.colorAt(x: x, y: y)?.alphaComponent ?? 0
                    alphaMass += max(a, b); deltaMass += abs(a-b); largestDelta = max(largestDelta, abs(a-b))
                }
            }
            // Native PDFKit's InkList writer rounds page coordinates. At 640%
            // that can shift an antialiased edge by a fraction of one pixel,
            // but must not turn the stroke into a lower-resolution bitmap.
            // The existing PDF save-path oracle permits 0.02 page points,
            // i.e. 0.128 screen pixels here. Bound both individual edge coverage
            // and total ink mass, rather than accepting an entire blurred stroke.
            XCTAssertLessThan(deltaMass / max(1, alphaMass), 0.02)
            XCTAssertLessThan(largestDelta, 0.15)
        }
    }

    func testCommittedStrokeKeepsTheSameHighZoomVectorPixelsAsTheLivePreview() throws {
        let view = InkPDFView(frame: CGRect(x: 0, y: 0, width: 900, height: 700))
        let document = PDFDocument(), page = PDFPage()
        page.setBounds(CGRect(x: 0, y: 0, width: 300, height: 300), for: .mediaBox)
        document.insert(page, at: 0)
        view.document = document; view.autoScales = false; view.scaleFactor = 4.7
        view.layoutDocumentView(); view.inkEnabled = true
        view.inkStyle = AnnotationStyle(strokeColor: .blue, fillColor: nil, lineWidth: 2)
        // Locate the curve in the visible part of a highly zoomed page.
        let visible = view.convert(page.bounds(for: .cropBox), from: page).intersection(view.bounds)
        let locations = [(0.2,0.4),(0.35,0.65),(0.5,0.35),(0.7,0.6)]
        let points = locations.map { view.convert(CGPoint(x: visible.minX + visible.width * $0.0,
                                                         y: visible.minY + visible.height * $0.1), to: page) }
        view.beginGesture(on: page, at: points[0])
        for point in points.dropFirst() { view.updateGesture(to: point) }
        let preview = try XCTUnwrap(view.renderShapeOverlay())
        try recordEvidence(preview, name: "ink-live-preview-470.png")
        view.onInk = { points, _, style in
            let path = InkStrokePath.make(points: points)
            let bounds = path.boundingBoxOfPath.insetBy(dx: -style.lineWidth, dy: -style.lineWidth)
            var offset = CGAffineTransform(translationX: -bounds.minX, y: -bounds.minY)
            let annotation = PDFAnnotation(bounds: bounds, forType: .ink, withProperties: nil)
            annotation.color = style.strokeColor
            let border = PDFBorder(); border.lineWidth = style.lineWidth; annotation.border = border
            annotation.add(InkStrokePath.bezier(path.copy(using: &offset)!))
            page.addAnnotation(annotation)
        }
        view.finishGesture()
        XCTAssertFalse(view.hasInkPreview)
        let committed = try XCTUnwrap(view.renderShapeOverlay())
        try recordEvidence(committed, name: "ink-committed-470.png")
        XCTAssertEqual(preview, committed,
                       "Releasing the pen must retain the same sharp rendered pixels, not hand off to a scaled native bitmap")
    }
}
