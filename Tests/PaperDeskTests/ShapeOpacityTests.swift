import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor final class ShapeOpacityTests: XCTestCase {
    private func controls(_ view: NSView) -> [NSControl] {
        (view as? NSControl).map { [$0] } ?? view.subviews.flatMap(controls)
    }
    func testNewInkTransparencySurvivesNativePDFRoundTrip() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        for smoothed in [false, true] {
            try workspace.addInk(points: [CGPoint(x: 70, y: 100), CGPoint(x: 150, y: 160), CGPoint(x: 200, y: 110)], page: 0, color: NSColor.blue.withAlphaComponent(0.4), width: 3, smoothed: smoothed)
        }
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".pdf")
        defer { try? FileManager.default.removeItem(at: url) }; try workspace.saveCopy(to: url)
        let document = try XCTUnwrap(PDFDocument(url: url))
        let page = try XCTUnwrap(document.page(at: 0))
        for annotation in page.annotations {
            XCTAssertEqual(AnnotationStyle(annotation: annotation).strokeColor.alphaComponent, 0.4, accuracy: 0.001)
        }
    }
    func testTransparencyFieldUpdatesBothColorsAndRejectsInvalidInput() throws {
        let bar = ShapeStyleBarView()
        bar.configure(tool: .rectangle, style: AnnotationStyle(strokeColor: .red, fillColor: .yellow, lineWidth: 3))
        let field = try XCTUnwrap(controls(bar).first { $0.accessibilityLabel() == "透明度，百分比" } as? NSTextField)
        var commits = 0; bar.onStyleChange = { _ in commits += 1 }
        field.stringValue = "37"; _ = field.sendAction(field.action, to: field.target)
        XCTAssertEqual(bar.style.strokeColor.alphaComponent, 0.63, accuracy: 0.001)
        XCTAssertEqual(try XCTUnwrap(bar.style.fillColor).alphaComponent, 0.63, accuracy: 0.001)
        XCTAssertEqual(commits, 1)
        for invalid in ["-1", "101", "nan", "abc"] {
            field.stringValue = invalid; _ = field.sendAction(field.action, to: field.target)
            XCTAssertEqual(bar.style.strokeColor.alphaComponent, 0.63, accuracy: 0.001)
        }
        XCTAssertEqual(commits, 1)
    }

    func testTransparencySliderPreviewsThenCommitsOnce() throws {
        let bar = ShapeStyleBarView()
        let slider = try XCTUnwrap(controls(bar).first { $0.accessibilityLabel() == "透明度滑块" } as? ShapeWidthSlider)
        var previews = 0, commits = 0
        bar.onStylePreview = { _ in previews += 1 }; bar.onStyleChange = { _ in commits += 1 }
        slider.beginShapeTracking()
        for value in [20.0, 40.0, 65.0] { slider.doubleValue = value; _ = slider.sendAction(slider.action, to: slider.target) }
        XCTAssertEqual(previews, 3); XCTAssertEqual(commits, 0)
        slider.endShapeTracking(); XCTAssertEqual(commits, 1)
        XCTAssertEqual(bar.style.strokeColor.alphaComponent, 0.35, accuracy: 0.001)
    }

    func testSelectedShapeTransparencyIsUndoableAndSurvivesSaveReopen() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation); c.addRectangle()
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        c.pdfView.beginGesture(on: page, at: CGPoint(x: 70, y: 100)); c.pdfView.updateGesture(to: CGPoint(x: 170, y: 180)); c.pdfView.finishGesture()
        c.changeShapeStyle(AnnotationStyle(strokeColor: .red, fillColor: .red, lineWidth: 3))
        let field = try XCTUnwrap(controls(c.shapeStyleBar).first { $0.accessibilityLabel() == "透明度，百分比" } as? NSTextField)
        field.stringValue = "65"; _ = field.sendAction(field.action, to: field.target)
        let a = try XCTUnwrap(c.workspace.document?.page(at: 0)?.annotations.first)
        XCTAssertEqual(a.color.alphaComponent, 0.35, accuracy: 0.001)
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".pdf")
        defer { try? FileManager.default.removeItem(at: url) }
        try c.workspace.saveCopy(to: url)
        let reopenedDocument = try XCTUnwrap(PDFDocument(url: url))
        let reopened = try XCTUnwrap(reopenedDocument.page(at: 0)?.annotations.first)
        XCTAssertEqual(AnnotationStyle(annotation: reopened).strokeColor.alphaComponent, 0.35, accuracy: 0.001)
        c.undo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.color.alphaComponent, 1)
        let another = WorkbenchController(); _ = another.view
        XCTAssertTrue(another.open(url: url)); another.pdfView.onAnnotationSelection?(0, 0)
        let reopenedField = try XCTUnwrap(controls(another.shapeStyleBar).first { $0.accessibilityLabel() == "透明度，百分比" } as? NSTextField)
        XCTAssertEqual(reopenedField.doubleValue, 65, accuracy: 0.001)
        let savedDocument = try XCTUnwrap(PDFDocument(url: url))
        let savedPage = try XCTUnwrap(savedDocument.page(at: 0))
        let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 612, pixelsHigh: 792, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        let context = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: bitmap))
        NSGraphicsContext.saveGraphicsState(); NSGraphicsContext.current = context
        NSColor.white.setFill(); CGRect(x: 0, y: 0, width: 612, height: 792).fill()
        savedPage.draw(with: .mediaBox, to: context.cgContext)
        NSGraphicsContext.restoreGraphicsState()
        let pixel = try XCTUnwrap(bitmap.colorAt(x: 120, y: 792 - 140)?.usingColorSpace(.deviceRGB))
        XCTAssertEqual(pixel.greenComponent, 0.65, accuracy: 0.05, "Saved native PDF must actually render semi-transparent fill")
        reopenedField.stringValue = "0"; _ = reopenedField.sendAction(reopenedField.action, to: reopenedField.target)
        XCTAssertEqual(reopenedField.doubleValue, 0, accuracy: 0.001, "A restored shape can return to fully opaque")
        XCTAssertEqual(another.shapeStyleBar.style.strokeColor.alphaComponent, 1, accuracy: 0.001)
    }
}
