import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor final class FreeTextRenderingSharpnessTests: XCTestCase {
    private func fixture(rotation: Int = 0, scale: CGFloat = 1.78) throws -> (InkPDFView, PDFAnnotation) {
        let document = PDFDocument(), page = PDFPage()
        page.setBounds(CGRect(x: 20, y: 30, width: 300, height: 300), for: .mediaBox)
        page.rotation = rotation
        let annotation = PDFAnnotation(bounds: CGRect(x: 85, y: 145, width: 180, height: 90), forType: .freeText, withProperties: nil)
        annotation.contents = "自由文字 Sharp 123\n输入以后仍然清晰"
        annotation.font = NSFont(name: "Helvetica-Bold", size: 18)
        annotation.fontColor = .black; annotation.color = .clear
        annotation.setValue(rotation, forAnnotationKey: PDFAnnotationKey(rawValue: "/Rotate"))
        page.addAnnotation(annotation); document.insert(page, at: 0)
        let saved = try XCTUnwrap(document.dataRepresentation())
        let view = InkPDFView(frame: CGRect(x: 0, y: 0, width: 900, height: 700))
        view.document = try XCTUnwrap(PDFDocument(data: saved))
        view.autoScales = false; view.scaleFactor = scale; view.layoutDocumentView()
        let displayedPage = try XCTUnwrap(view.document?.page(at: 0))
        let displayed = try XCTUnwrap(displayedPage.annotations.first)
        view.go(to: displayed.bounds, on: displayedPage); view.layout()
        return (view, displayed)
    }

    private func paintedPixels(_ data: Data) throws -> Int {
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: data))
        var painted = 0
        for y in 0..<bitmap.pixelsHigh {
            for x in 0..<bitmap.pixelsWide where (bitmap.colorAt(x: x, y: y)?.alphaComponent ?? 0) > 0.01 { painted += 1 }
        }
        return painted
    }

    func testRetinaAppearanceIsRasterizedAtDevicePixelsNotMagnifiedFromPagePixels() throws {
        let (view, _) = try fixture(scale: 3.2)
        let overlay = try XCTUnwrap(view.subviews.first { $0.layer?.zPosition == 1000 })
        func deviceLayers(in layer: CALayer) -> [CALayer] {
            var found: [CALayer] = layer is FreeTextAppearanceLayer && layer.contentsScale >= 2 && layer.bounds.width > 0 ? [layer] : []
            for child in layer.sublayers ?? [] { found += deviceLayers(in: child) }
            return found
        }
        XCTAssertEqual(deviceLayers(in: try XCTUnwrap(overlay.layer)).count, 1)
        let retina = try XCTUnwrap(view.renderShapeOverlay(backingScale: 2))
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: retina))
        XCTAssertEqual(bitmap.pixelsWide, 1800)
        XCTAssertEqual(bitmap.pixelsHigh, 1400)
        XCTAssertGreaterThan(try paintedPixels(retina), 4000)
    }

    func testHiddenSourceStaysHiddenAndRemovingTextClearsOnlyPresentationPixels() throws {
        let (view, annotation) = try fixture()
        let document = try XCTUnwrap(view.document), page = try XCTUnwrap(document.page(at: 0))
        let hidden = PDFAnnotation(bounds: annotation.bounds, forType: .freeText, withProperties: nil)
        hidden.contents = "must remain hidden"; hidden.font = .systemFont(ofSize: 18)
        hidden.shouldDisplay = false; page.addAnnotation(hidden)
        view.refreshCommittedInkRendering()
        let painted = try XCTUnwrap(view.renderShapeOverlay())
        let saved = try XCTUnwrap(view.presentationDataRepresentation())
        let reopened = try XCTUnwrap(PDFDocument(data: saved)?.page(at: 0))
        XCTAssertTrue(try XCTUnwrap(reopened.annotations.first).shouldDisplay)
        XCTAssertFalse(try XCTUnwrap(reopened.annotations.last).shouldDisplay)
        page.removeAnnotation(annotation); view.refreshCommittedInkRendering()
        XCTAssertEqual(try paintedPixels(XCTUnwrap(view.renderShapeOverlay())), 0)
        XCTAssertTrue(annotation.shouldDisplay)
        XCTAssertTrue(view.document === document)
        page.addAnnotation(annotation); view.refreshCommittedInkRendering()
        XCTAssertEqual(painted, view.renderShapeOverlay())
        view.document = nil
        XCTAssertFalse(hidden.shouldDisplay)
    }

    func testEditingExistingTextHidesCommittedAppearanceUntilCancel() throws {
        let (view, annotation) = try fixture()
        let page = try XCTUnwrap(view.document?.page(at: 0))
        let before = try XCTUnwrap(view.renderShapeOverlay())
        let session = PDFInlineTextSession(pdfView: view, page: page, bounds: annotation.bounds,
            text: annotation.contents ?? "", font: try XCTUnwrap(annotation.font), color: .black,
            annotation: annotation, commit: { _, _, _, _ in }, failure: { XCTFail($0.localizedDescription) })
        view.inlineTextSession = session; view.refreshCommittedInkRendering()
        XCTAssertEqual(try paintedPixels(XCTUnwrap(view.renderShapeOverlay())), 0, "No duplicate ghost under the native editor")
        XCTAssertTrue(session.finish(commit: false))
        XCTAssertEqual(before, view.renderShapeOverlay())
    }

    func testNativeAppearanceDrawIsVectorAndDoesNotDependOnDisplayFlag() throws {
        let (_, annotation) = try fixture()
        var images: [Data] = []
        for display in [true, false] {
            annotation.shouldDisplay = display
            let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 1200, pixelsHigh: 1200,
                bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
            let context = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: bitmap)?.cgContext)
            context.scaleBy(x: 4, y: 4)
            annotation.draw(with: .cropBox, in: context)
            let image = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            XCTAssertGreaterThan(try paintedPixels(image), 1000)
            images.append(image)
        }
        XCTAssertEqual(images[0], images[1], "Direct /AP drawing must bypass PDFKit's low-resolution screen cache without mutating display flags")
    }

    func testCommittedFreeTextUsesSharpAppearanceSurfaceAtEveryZoomAndRotation() throws {
        for rotation in [0, 90, 180, 270] {
            let (view, annotation) = try fixture(rotation: rotation, scale: 1.78)
            XCTAssertFalse(annotation.shouldDisplay, "Do not hand committed text to PDFKit's page-resolution annotation bitmap")
            let first = try XCTUnwrap(view.renderShapeOverlay())
            XCTAssertGreaterThan(try paintedPixels(first), 1000, "The committed native text appearance must be painted on the sharp surface")
            view.scaleFactor = 3.2; view.layoutDocumentView(); view.layout()
            let zoomed = try XCTUnwrap(view.renderShapeOverlay())
            XCTAssertNotEqual(first, zoomed, "Zoom must redraw the original appearance rather than scale an old bitmap")
            let saved = try XCTUnwrap(view.presentationDataRepresentation())
            let reopened = try XCTUnwrap(PDFDocument(data: saved)?.page(at: 0)?.annotations.first)
            XCTAssertTrue(reopened.shouldDisplay)
            XCTAssertEqual(reopened.contents, annotation.contents)
            XCTAssertEqual(reopened.font?.pointSize, annotation.font?.pointSize)
            view.document = nil
            XCTAssertTrue(annotation.shouldDisplay, "Detaching the reader must restore its presentation flags")
        }
    }
}
