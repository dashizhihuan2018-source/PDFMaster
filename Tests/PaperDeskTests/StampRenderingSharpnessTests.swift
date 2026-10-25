import AppKit
import PDFKit
import XCTest
import CryptoKit
@testable import PaperDesk

@MainActor final class StampRenderingSharpnessTests: XCTestCase {
    private final class VectorStamp: PDFAnnotation {
        override func draw(with box: PDFDisplayBox, in context: CGContext) {
            context.saveGState(); defer { context.restoreGState() }
            page?.transform(context, for: box)
            context.setFillColor(NSColor.white.cgColor); context.fill(bounds)
            context.setStrokeColor(NSColor.black.cgColor); context.setLineWidth(0.3)
            for offset in stride(from: CGFloat(0), to: bounds.width, by: 2) {
                context.move(to: CGPoint(x: bounds.minX + offset, y: bounds.minY))
                context.addLine(to: CGPoint(x: bounds.minX + offset, y: bounds.maxY))
            }
            context.strokePath()
        }
    }

    private func fixture(rotation: Int = 0) -> (InkPDFView, PDFPage, PDFAnnotation) {
        let page = PDFPage(), document = PDFDocument()
        page.setBounds(CGRect(x: 20, y: 30, width: 300, height: 300), for: .mediaBox)
        page.rotation = rotation
        let stamp = VectorStamp(bounds: CGRect(x: 90, y: 130, width: 140, height: 100), forType: .stamp, withProperties: nil)
        page.addAnnotation(stamp); document.insert(page, at: 0)
        let view = InkPDFView(frame: CGRect(x: 0, y: 0, width: 700, height: 600))
        view.document = document; view.autoScales = false; view.scaleFactor = 2.3
        view.layoutDocumentView(); view.go(to: stamp.bounds, on: page); view.layout()
        return (view, page, stamp)
    }

    private func appearanceLayers(_ view: InkPDFView) throws -> [CALayer] {
        let overlay = try XCTUnwrap(view.subviews.first { $0.layer?.zPosition == 1000 }?.layer)
        func visit(_ layer: CALayer) -> [CALayer] {
            (layer is FreeTextAppearanceLayer || layer is CAShapeLayer ? [layer] : []) + (layer.sublayers ?? []).flatMap(visit)
        }
        return visit(overlay)
    }

    func testStampUsesVisibleDevicePixelAppearanceAndRedrawsAtZoomForEveryRotation() throws {
        for rotation in [0, 90, 180, 270] {
            let (view, _, stamp) = fixture(rotation: rotation)
            XCTAssertFalse(stamp.shouldDisplay, "Do not magnify PDFKit's page-resolution Stamp cache")
            let layers = try appearanceLayers(view).filter { $0 is FreeTextAppearanceLayer }
            XCTAssertEqual(layers.count, 1)
            if let layer = layers.first {
                XCTAssertGreaterThanOrEqual(layer.contentsScale, 2)
                XCTAssertLessThanOrEqual(layer.bounds.width, view.bounds.width)
                XCTAssertLessThanOrEqual(layer.bounds.height, view.bounds.height)
            }
            let before = try XCTUnwrap(view.renderShapeOverlay(backingScale: 2))
            view.scaleFactor = 4.1; view.layoutDocumentView(); view.layout()
            XCTAssertNotEqual(before, view.renderShapeOverlay(backingScale: 2))
        }
    }

    func testStampInkAndLineRetainPDFAnnotationStackOrder() throws {
        let (view, page, _) = fixture()
        let ink = PDFAnnotation(bounds: CGRect(x: 100, y: 140, width: 60, height: 50), forType: .ink, withProperties: nil)
        let path = NSBezierPath(); path.move(to: .zero); path.line(to: CGPoint(x: 40, y: 30)); ink.add(path)
        page.addAnnotation(ink)
        let line = PDFAnnotation(bounds: ink.bounds, forType: .line, withProperties: nil)
        line.startPoint = .zero; line.endPoint = CGPoint(x: 40, y: 30); page.addAnnotation(line)
        view.refreshCommittedInkRendering()
        let layers = try appearanceLayers(view)
        let appearances = layers.filter { $0 is FreeTextAppearanceLayer }
        let inkLayer = try XCTUnwrap(layers.first { $0 is CAShapeLayer && $0.pathForTesting != nil && $0.zPosition == 1 })
        XCTAssertEqual(appearances.count, 2)
        XCTAssertTrue(appearances.allSatisfy { $0.superlayer === inkLayer.superlayer })
        XCTAssertEqual(appearances.map(\.zPosition).sorted(), [0, 2])
        XCTAssertFalse(line.shouldDisplay)
    }

    func testHiddenStampStaysHiddenAndSerializationRestoresOnlyPresentationSuppression() throws {
        let (view, page, stamp) = fixture()
        let hidden = VectorStamp(bounds: stamp.bounds, forType: .stamp, withProperties: nil)
        hidden.shouldDisplay = false; page.addAnnotation(hidden); view.refreshCommittedInkRendering()
        XCTAssertEqual(try appearanceLayers(view).filter { $0 is FreeTextAppearanceLayer }.count, 1)
        let saved = try XCTUnwrap(view.presentationDataRepresentation())
        let annotations = try XCTUnwrap(PDFDocument(data: saved)?.page(at: 0)?.annotations)
        XCTAssertTrue(annotations[0].shouldDisplay); XCTAssertFalse(annotations[1].shouldDisplay)
        XCTAssertFalse(stamp.shouldDisplay)
        page.removeAnnotation(stamp); view.refreshCommittedInkRendering()
        XCTAssertTrue(stamp.shouldDisplay)
        XCTAssertEqual(try appearanceLayers(view).filter { $0 is FreeTextAppearanceLayer }.count, 0)
        view.document = nil; XCTAssertFalse(hidden.shouldDisplay)
    }

    func testNativeStampAppearanceKeepsSubPointVectorDetailsAtRetinaZoom() throws {
        let commands = "1 1 1 rg 0 0 100 100 re f 0 0 0 RG 0.15 w\n" +
            stride(from: 5, to: 95, by: 1).map { "\($0) 5 m \($0) 95 l S" }.joined(separator: "\n")
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [20 30 220 230] /CropBox [30 40 210 220] /Resources << >> /Contents 6 0 R /Annots [4 0 R] >>",
            "<< /Type /Annot /Subtype /Stamp /Rect [70 80 170 180] /F 4 /AP << /N 5 0 R >> >>",
            "<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << >> /Length \(commands.utf8.count) >>\nstream\n\(commands)\nendstream",
            "<< /Length 0 >>\nstream\n\nendstream"
        ]
        var pdf = "%PDF-1.7\n", offsets = [0]
        for (index, object) in objects.enumerated() {
            offsets.append(pdf.utf8.count); pdf += "\(index+1) 0 obj\n\(object)\nendobj\n"
        }
        let xref = pdf.utf8.count
        pdf += "xref\n0 \(objects.count+1)\n0000000000 65535 f \n"
        for offset in offsets.dropFirst() { pdf += String(format: "%010d 00000 n \n", offset) }
        pdf += "trailer\n<< /Size \(objects.count+1) /Root 1 0 R >>\nstartxref\n\(xref)\n%%EOF\n"
        for rotation in [0, 90, 180, 270] {
            let document = try XCTUnwrap(PDFDocument(data: Data(pdf.utf8)))
            let page = try XCTUnwrap(document.page(at: 0)), stamp = try XCTUnwrap(page.annotations.first)
            page.rotation = rotation
            let view = InkPDFView(frame: CGRect(x: 0, y: 0, width: 650, height: 650))
            view.document = document; view.autoScales = false; view.scaleFactor = 2
            view.layoutDocumentView(); view.go(to: stamp.bounds, on: page); view.layout()
            XCTAssertFalse(stamp.shouldDisplay)
            let bitmap = try XCTUnwrap(NSBitmapImageRep(data: XCTUnwrap(view.renderShapeOverlay(backingScale: 2))))
            // Scan along the original page x axis, rotated into the viewport.
            // Each 1pt interval has four device pixels, even with nonzero crop.
            let start = view.convert(CGPoint(x: stamp.bounds.minX + 2, y: stamp.bounds.midY), from: page)
            let end = view.convert(CGPoint(x: stamp.bounds.maxX - 2, y: stamp.bounds.midY), from: page)
            var darkRuns = 0, wasDark = false
            for step in 0..<384 {
                let t = CGFloat(step) / 384
                let x = Int((start.x + (end.x-start.x)*t) * 2)
                let y = Int((view.bounds.height - start.y - (end.y-start.y)*t) * 2)
                let color = try XCTUnwrap(bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB))
                let dark = color.alphaComponent > 0.5 && color.redComponent < 0.8
                if dark && !wasDark { darkRuns += 1 }; wasDark = dark
            }
            XCTAssertGreaterThanOrEqual(darkRuns, 85, "Native /AP fine strokes, crop origin and \(rotation)° orientation")
        }
    }

    func testLocalReproductionPDFWithoutChangingSourceBytes() throws {
        guard let path = ProcessInfo.processInfo.environment["PDFMASTER_SHARPNESS_FIXTURE"],
              let output = ProcessInfo.processInfo.environment["PDFMASTER_SHARPNESS_EVIDENCE"] else {
            throw XCTSkip("Opt-in local user PDF; never package the user's file as a fixture")
        }
        let url = URL(fileURLWithPath: path), before = try Data(contentsOf: url)
        let document = try XCTUnwrap(PDFDocument(data: before))
        let view = InkPDFView(frame: CGRect(x: 0, y: 0, width: 1200, height: 900))
        view.document = document; view.autoScales = false; view.scaleFactor = 1.3
        view.layoutDocumentView(); view.layout()
        let stamps = try XCTUnwrap(document.page(at: 0)).annotations.filter { $0.type == "Stamp" }
        XCTAssertFalse(stamps.isEmpty)
        XCTAssertTrue(stamps.allSatisfy { !$0.shouldDisplay })
        let first = try XCTUnwrap(view.renderShapeOverlay(backingScale: 2))
        try FileManager.default.createDirectory(at: URL(fileURLWithPath: output), withIntermediateDirectories: true)
        try first.write(to: URL(fileURLWithPath: output).appendingPathComponent("actual-file-sharp-overlay-130-retina.png"))
        let saved = try XCTUnwrap(view.presentationDataRepresentation())
        let reopened = try XCTUnwrap(PDFDocument(data: saved))
        XCTAssertTrue(try XCTUnwrap(reopened.page(at: 0)).annotations.allSatisfy(\.shouldDisplay))
        XCTAssertEqual(SHA256.hash(data: before), SHA256.hash(data: try Data(contentsOf: url)))
    }
}

private extension CALayer {
    var pathForTesting: CGPath? { (self as? CAShapeLayer)?.path }
}
