import AppKit
import PDFKit
import XCTest
import PaperCore
@testable import PaperDesk

@MainActor final class InkStyleBarTests: XCTestCase {
    private func fire(_ control: NSControl) { _ = control.sendAction(control.action, to: control.target) }

    func testInlinePenColorWidthAndPresetsDoNotChangeShapeDefaultsOrDocument() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation); c.addRectangle()
        c.changeShapeStyle(AnnotationStyle(strokeColor: .red, fillColor: .yellow, lineWidth: 3))
        c.toggleInk(NSMenuItem())
        let revision = c.workspace.revision
        XCTAssertFalse(c.inkStyleBar.isHidden)
        XCTAssertTrue(c.shapeStyleBar.isHidden)
        XCTAssertTrue(c.inkStyleBar.toolChooser.isHidden)
        XCTAssertTrue(c.inkStyleBar.fillEnabledButton.isHidden)
        c.inkStyleBar.strokeColorWell.color = .blue; fire(c.inkStyleBar.strokeColorWell)
        c.inkStyleBar.widthPresets.selectItem(at: 5); fire(c.inkStyleBar.widthPresets)
        XCTAssertEqual(c.pdfView.inkStyle.lineWidth, 8)
        XCTAssertEqual(c.pdfView.inkStyle.strokeColor, .blue)
        XCTAssertEqual(c.workspace.revision, revision)
        c.addRectangle()
        XCTAssertTrue(c.inkStyleBar.isHidden)
        XCTAssertEqual(c.shapeStyleBar.style.lineWidth, 3)
        XCTAssertEqual(c.shapeStyleBar.style.strokeColor, .red)
        XCTAssertEqual(c.shapeStyleBar.style.fillColor, .yellow)
        c.toggleInk(NSMenuItem())
        XCTAssertEqual(c.inkStyleBar.style.lineWidth, 8)
        XCTAssertEqual(c.inkStyleBar.style.strokeColor, .blue)
    }

    func testWidthScrubAffectsNextStrokeWithoutMutationAndInvalidWidthRetainsSetting() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation); c.toggleInk(NSMenuItem())
        let revision = c.workspace.revision
        c.inkStyleBar.widthSlider.beginShapeTracking()
        for width in [3.0,5.0,8.0] { c.inkStyleBar.widthSlider.doubleValue = width; fire(c.inkStyleBar.widthSlider) }
        XCTAssertEqual(c.pdfView.inkStyle.lineWidth, 8)
        XCTAssertEqual(c.workspace.revision, revision)
        c.inkStyleBar.widthSlider.endShapeTracking()
        c.changeInkStyle(AnnotationStyle(strokeColor: .red, fillColor: nil, lineWidth: .infinity))
        XCTAssertEqual(c.pdfView.inkStyle.lineWidth, 8)
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        c.pdfView.beginGesture(on: page, at: CGPoint(x: 70,y: 100)); c.pdfView.updateGesture(to: CGPoint(x: 200,y: 180)); c.pdfView.finishGesture()
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.border?.lineWidth, 8)
    }

    func testPenOverridesContentModeAndEscClearsMenuAndStyleControls() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation)
        c.pdfView.contentEditingEnabled = true
        let menu = NSMenuItem(); c.toggleInk(menu)
        XCTAssertFalse(c.pdfView.contentEditingEnabled)
        XCTAssertEqual(menu.state, .on)
        XCTAssertTrue(c.pdfView.cancelPlacementForEscape())
        XCTAssertEqual(menu.state, .off)
        XCTAssertTrue(c.inkStyleBar.isHidden)
    }

    func testSelectedInkUsesInlineColorAndWidthWithPreviewOnlyScrubAndOneUndo() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation); c.toggleInk(NSMenuItem())
        let page = try XCTUnwrap(c.pdfView.document?.page(at:0))
        c.pdfView.beginGesture(on:page,at:CGPoint(x:70,y:100)); c.pdfView.updateGesture(to:CGPoint(x:170,y:180)); c.pdfView.finishGesture()
        c.resetDocumentInteractionState()
        c.pdfView.onAnnotationSelection?(0,0)
        c.editSelectedAnnotation() // Must route to inline controls, never run a modal.
        XCTAssertFalse(c.inkStyleBar.isHidden)
        XCTAssertEqual(c.inkStyleBar.mode,.selection)
        let labels = c.inkStyleBar.subviews.flatMap(\.subviews).compactMap { ($0 as? NSTextField)?.stringValue }
        XCTAssertTrue(labels.contains("所选笔迹"))
        let revision = c.workspace.revision
        let before = try XCTUnwrap(c.workspace.document?.page(at:0)?.annotations.first)
        let oldBounds = before.bounds
        let oldPoints = try XCTUnwrap(before.paths?.first).cgPath
        c.inkStyleBar.widthSlider.beginShapeTracking()
        for width in [3.0,5.0,8.0] { c.inkStyleBar.widthSlider.doubleValue = width; fire(c.inkStyleBar.widthSlider) }
        XCTAssertEqual(c.workspace.revision,revision)
        XCTAssertEqual(c.workspace.document?.page(at:0)?.annotations.first?.border?.lineWidth,2)
        c.inkStyleBar.widthSlider.endShapeTracking()
        XCTAssertEqual(c.workspace.revision,revision+1)
        let changed = try XCTUnwrap(c.workspace.document?.page(at:0)?.annotations.first)
        XCTAssertEqual(changed.border?.lineWidth,8)
        var oldOffset = CGAffineTransform(translationX:oldBounds.minX,y:oldBounds.minY)
        var newOffset = CGAffineTransform(translationX:changed.bounds.minX,y:changed.bounds.minY)
        let beforeAbsolute = try XCTUnwrap(oldPoints.copy(using: &oldOffset))
        let afterAbsolute = try XCTUnwrap(try XCTUnwrap(changed.paths?.first).cgPath.copy(using: &newOffset))
        // Undo isolation serializes PDF coordinates at native decimal precision.
        // Compare every vertex, rather than requiring bit-identical floating math.
        func elements(_ path: CGPath) -> [(CGPathElementType, [CGPoint])] {
            var result: [(CGPathElementType, [CGPoint])] = []
            path.applyWithBlock { pointer in
                let item = pointer.pointee
                let count: Int
                switch item.type { case .moveToPoint, .addLineToPoint: count = 1; case .addQuadCurveToPoint: count = 2; case .addCurveToPoint: count = 3; case .closeSubpath: count = 0; @unknown default: count = 0 }
                result.append((item.type, (0..<count).map { item.points[$0] }))
            }
            return result
        }
        let beforeElements = elements(beforeAbsolute), afterElements = elements(afterAbsolute)
        XCTAssertEqual(beforeElements.count, afterElements.count)
        for (before, after) in zip(beforeElements, afterElements) {
            XCTAssertEqual(before.0, after.0); XCTAssertEqual(before.1.count, after.1.count)
            for (left, right) in zip(before.1, after.1) {
                XCTAssertEqual(left.x, right.x, accuracy: 0.001)
                XCTAssertEqual(left.y, right.y, accuracy: 0.001)
            }
        }
        c.undo()
        XCTAssertEqual(c.workspace.document?.page(at:0)?.annotations.first?.border?.lineWidth,2)
        c.pdfView.onAnnotationSelection?(0,0)
        c.inkStyleBar.strokeColorWell.color = .red; fire(c.inkStyleBar.strokeColorWell)
        XCTAssertEqual(c.workspace.document?.page(at:0)?.annotations.first?.color.usingColorSpace(.deviceRGB),NSColor.red.usingColorSpace(.deviceRGB))
        c.toggleInk(NSMenuItem())
        XCTAssertEqual(c.pdfView.inkStyle.lineWidth,2,"Selected-object editing must not overwrite drawing defaults")
        XCTAssertEqual(c.pdfView.inkStyle.strokeColor,.systemIndigo)
    }

    func testNativeSelectedInkSliderPreviewsPixelsAndReleaseCommitsExactlyOnce() throws {
        let c = WorkbenchController(); _ = c.view
        let window = NSWindow(contentRect:NSRect(x:0,y:0,width:1280,height:820),styleMask:[.titled],backing:.buffered,defer:false)
        window.contentViewController = c; defer { window.contentViewController = nil }
        try c.workspace.createBlank()
        try c.workspace.addInk(points:[CGPoint(x:100,y:610),CGPoint(x:160,y:660),CGPoint(x:250,y:620)],page:0,color:.blue,width:2,smoothed:true)
        c.refreshDocument(); c.enterMode(.annotation); c.view.layoutSubtreeIfNeeded()
        let page = try XCTUnwrap(c.pdfView.document?.page(at:0))
        let hit = c.pdfView.convert(c.pdfView.convert(CGPoint(x:150,y:630),from:page),to:nil)
        let down = try XCTUnwrap(NSEvent.mouseEvent(with:.leftMouseDown,location:hit,modifierFlags:[],timestamp:0,windowNumber:window.windowNumber,context:nil,eventNumber:0,clickCount:1,pressure:1))
        c.pdfView.mouseDown(with:down)
        XCTAssertFalse(c.inkStyleBar.isHidden,"Native annotation hit test must select Ink")
        c.view.layoutSubtreeIfNeeded()
        let slider = c.inkStyleBar.widthSlider
        slider.displayIfNeeded()
        let cell = try XCTUnwrap(slider.cell as? NSSliderCell)
        let knob = cell.knobRect(flipped:slider.isFlipped)
        let start = CGPoint(x:knob.midX,y:knob.midY)
        func event(_ type:NSEvent.EventType,_ point:CGPoint)->NSEvent {
            NSEvent.mouseEvent(with:type,location:slider.convert(point,to:nil),modifierFlags:[],timestamp:0,windowNumber:window.windowNumber,context:nil,eventNumber:0,clickCount:1,pressure:1)!
        }
        let revision = c.workspace.revision
        let before = try XCTUnwrap(c.pdfView.renderShapeOverlay())
        slider.mouseDown(with:event(.leftMouseDown,start))
        XCTAssertTrue(slider.isShapeTracking)
        slider.mouseDragged(with:event(.leftMouseDragged,CGPoint(x:start.x+18,y:start.y)))
        XCTAssertTrue(c.pdfView.hasInkStylePreview)
        let previewPath = try XCTUnwrap(c.pdfView.inkPreviewPath)
        XCTAssertFalse(previewPath.isEmpty)
        XCTAssertTrue(c.pdfView.convert(previewPath.boundingBoxOfPath,from:page).intersects(c.pdfView.bounds))
        let artifact = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".png")
        defer { try? FileManager.default.removeItem(at:artifact) }
        try XCTUnwrap(c.pdfView.renderShapeOverlay()).write(to:artifact)
        XCTAssertNotEqual(before,try XCTUnwrap(c.pdfView.renderShapeOverlay()))
        let width = c.inkStyleBar.style.lineWidth
        XCTAssertEqual(c.workspace.revision,revision)
        slider.mouseUp(with:event(.leftMouseUp,CGPoint(x:start.x+18,y:start.y)))
        XCTAssertEqual(c.workspace.revision,revision+1)
        XCTAssertFalse(c.pdfView.hasInkStylePreview)
        XCTAssertEqual(c.workspace.document?.page(at:0)?.annotations.first?.border?.lineWidth ?? 0,width,accuracy:0.0001)
        slider.mouseUp(with:event(.leftMouseUp,CGPoint(x:start.x+18,y:start.y)))
        XCTAssertEqual(c.workspace.revision,revision+1)
        let saved = FileManager.default.temporaryDirectory.appendingPathComponent("pdfmaster-selected-ink-\(UUID().uuidString).pdf")
        defer { try? FileManager.default.removeItem(at:saved) }
        try c.workspace.saveCopy(to:saved)
        XCTAssertEqual(try XCTUnwrap(PDFDocument(url:saved)?.page(at:0)?.annotations.first).border?.lineWidth ?? 0,width,accuracy:0.001)
        c.undo(); XCTAssertEqual(c.workspace.document?.page(at:0)?.annotations.first?.border?.lineWidth,2)
    }

    func testSelectedInkScrubIsCancelledWhenModeOrDocumentChanges() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); try c.workspace.addInk(points:[CGPoint(x:70,y:100),CGPoint(x:170,y:180)],page:0,color:.blue,width:2)
        c.refreshDocument(); c.enterMode(.annotation); c.pdfView.onAnnotationSelection?(0,0)
        let revision = c.workspace.revision
        c.inkStyleBar.widthSlider.beginShapeTracking()
        c.inkStyleBar.widthSlider.doubleValue = 8; fire(c.inkStyleBar.widthSlider)
        XCTAssertTrue(c.pdfView.hasInkStylePreview)
        c.enterMode(.editing)
        c.inkStyleBar.widthSlider.endShapeTracking()
        XCTAssertEqual(c.workspace.revision,revision)
        XCTAssertFalse(c.pdfView.hasInkStylePreview)
        c.enterMode(.annotation); c.pdfView.onAnnotationSelection?(0,0)
        c.inkStyleBar.widthSlider.beginShapeTracking(); c.inkStyleBar.widthSlider.doubleValue = 8; fire(c.inkStyleBar.widthSlider)
        try c.workspace.createBlank(); c.refreshDocument()
        let replacementRevision = c.workspace.revision
        c.inkStyleBar.widthSlider.endShapeTracking()
        XCTAssertFalse(c.pdfView.hasInkStylePreview)
        XCTAssertTrue(c.inkStyleBar.isHidden)
        XCTAssertEqual(c.workspace.revision,replacementRevision)
        XCTAssertEqual(c.workspace.document?.page(at:0)?.annotations.count,0)
    }

    func testCoreSelectedInkRejectsOtherTypesAndInvalidStyleWithoutHistoryChange() throws {
        let w = PDFWorkspace(); try w.createBlank()
        try w.addShape(.rectangle,page:0,bounds:CGRect(x:50,y:60,width:70,height:80),style:.default)
        let revision = w.revision
        XCTAssertThrowsError(try w.updateInkStyle(page:0,index:0,style:.default))
        XCTAssertEqual(w.revision,revision)
        try w.addInk(points:[CGPoint(x:70,y:100),CGPoint(x:170,y:180)],page:0,color:.blue,width:2)
        let inkRevision = w.revision
        for width in [CGFloat.nan, .infinity, 0, 33] {
            XCTAssertThrowsError(try w.updateInkStyle(page:0,index:1,style:AnnotationStyle(strokeColor:.red,fillColor:nil,lineWidth:width)))
            XCTAssertEqual(w.revision,inkRevision)
        }
    }

    func testSelectedInkSnapshotPreservesMultiPathMetadataThroughSaveAndReopen() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank()
        try c.workspace.cropPage(at:0,to:CGRect(x:37,y:53,width:400,height:500)); try c.workspace.rotate(page:0,degrees:90)
        try c.workspace.addInk(points:[CGPoint(x:100,y:180),CGPoint(x:160,y:230)],page:0,color:.blue,width:2)
        let original = try XCTUnwrap(c.workspace.document?.page(at:0)?.annotations.first)
        let extra = NSBezierPath(); extra.move(to:CGPoint(x:30,y:20)); extra.line(to:CGPoint(x:80,y:35)); original.add(extra)
        original.contents = "self-generated ink fixture"; original.userName = "test author"
        func globalPaths(_ annotation:PDFAnnotation) -> [CGPath] {
            var offset = CGAffineTransform(translationX:annotation.bounds.minX,y:annotation.bounds.minY)
            return (annotation.paths ?? []).compactMap { $0.cgPath.copy(using:&offset) }
        }
        let expected = globalPaths(original)
        c.refreshDocument(); c.enterMode(.annotation); c.pdfView.onAnnotationSelection?(0,0)
        let revision = c.workspace.revision
        c.previewInkStyle(AnnotationStyle(strokeColor:.red,fillColor:nil,lineWidth:8))
        XCTAssertTrue(c.pdfView.hasInkStylePreview)
        let combined = CGMutablePath(); expected.forEach { combined.addPath($0) }
        XCTAssertEqual(c.pdfView.inkPreviewPath,combined)
        XCTAssertEqual(c.workspace.revision,revision)
        c.changeInkStyle(AnnotationStyle(strokeColor:.red,fillColor:nil,lineWidth:8))
        let changed = try XCTUnwrap(c.workspace.document?.page(at:0)?.annotations.first)
        XCTAssertEqual(globalPaths(changed),expected)
        XCTAssertEqual(changed.contents,"self-generated ink fixture"); XCTAssertEqual(changed.userName,"test author")
        let reopened = try XCTUnwrap(PDFDocument(data:try XCTUnwrap(c.workspace.document?.dataRepresentation()))?.page(at:0)?.annotations.first)
        XCTAssertEqual(globalPaths(reopened),expected)
        XCTAssertEqual(reopened.paths?.count,2)
        XCTAssertEqual(reopened.contents,"self-generated ink fixture")
        XCTAssertEqual(reopened.border?.lineWidth,8)
    }
}
