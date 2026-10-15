import XCTest
import AppKit
import PDFKit
import PaperCore
@testable import PaperDesk

@MainActor final class ShapeInteractionTests: XCTestCase {
    func testDragPreviewDoesNotMutateUntilReleaseAndEscapeCancels() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation)
        c.addRectangle()
        XCTAssertTrue(c.pdfView.placementEnabled)
        XCTAssertFalse(c.shapeStyleBar.isHidden)
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        c.pdfView.beginGesture(on: page, at: CGPoint(x: 70, y: 100))
        c.pdfView.updateGesture(to: CGPoint(x: 170, y: 180))
        XCTAssertTrue(c.pdfView.hasShapePreview)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        XCTAssertTrue(c.pdfView.cancelPlacementForEscape())
        XCTAssertFalse(c.pdfView.hasShapePreview)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
    }

    func testDrawAndImmediateWidthEditAndMoveAreUndoable() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation); c.addEllipse()
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        c.pdfView.beginGesture(on: page, at: CGPoint(x: 70, y: 100))
        c.pdfView.updateGesture(to: CGPoint(x: 170, y: 180)); c.pdfView.finishGesture()
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.type, "Circle")
        c.changeShapeStyle(AnnotationStyle(strokeColor: .blue, fillColor: .yellow, lineWidth: 5))
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.border?.lineWidth, 5)
        c.undo()
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.border?.lineWidth, 2)
    }

    func testContinuousShapeModeAndNoCrossPageOrZeroSizeCommit() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation); c.addRectangle()
        let p = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        c.pdfView.beginGesture(on: p, at: CGPoint(x: 70, y: 100)); c.pdfView.finishGesture()
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        XCTAssertTrue(c.pdfView.placementEnabled)
        c.pdfView.beginGesture(on: p, at: CGPoint(x: 70, y: 100))
        c.pdfView.updateGesture(to: CGPoint(x: 2000, y: 2000)); c.pdfView.finishGesture()
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
        XCTAssertTrue(c.pdfView.placementEnabled)
    }

    func testWidthScrubbingPreviewsWithoutRepeatedUndoTransactions() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation); c.addRectangle()
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        c.pdfView.beginGesture(on: page, at: CGPoint(x: 70, y: 100)); c.pdfView.updateGesture(to: CGPoint(x: 170, y: 180)); c.pdfView.finishGesture()
        let revision = c.workspace.revision
        c.previewShapeStyle(AnnotationStyle(strokeColor: .blue, fillColor: nil, lineWidth: 8))
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertTrue(c.pdfView.hasShapePreview)
        c.changeShapeStyle(AnnotationStyle(strokeColor: .blue, fillColor: nil, lineWidth: 8))
        XCTAssertEqual(c.workspace.revision, revision + 1)
        XCTAssertFalse(c.pdfView.hasShapePreview)
        c.undo()
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.border?.lineWidth, 2)
    }

    private func event(_ kind: NSEvent.EventType, at point: CGPoint, in view: InkPDFView) -> NSEvent {
        NSEvent.mouseEvent(with: kind, location: view.convert(point, to: nil), modifierFlags: [], timestamp: 0, windowNumber: view.window?.windowNumber ?? 0, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
    }

    func testSelectingShapeDuringPageReflowDoesNotMoveItOrCommitUndo() throws {
        let view = InkPDFView(frame: NSRect(x: 0, y: 0, width: 800, height: 650))
        let window = NSWindow(contentRect: view.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = view
        let document = PDFDocument(), page = PDFPage()
        page.setBounds(CGRect(x: 0, y: 0, width: 600, height: 800), for: .mediaBox)
        document.insert(page, at: 0); view.document = document; view.scaleFactor = 0.7; view.layoutDocumentView()
        let original = CGRect(x: 180, y: 300, width: 150, height: 100)
        let annotation = PDFAnnotation(bounds: original, forType: .square, withProperties: nil)
        page.addAnnotation(annotation)
        var commits = 0
        view.onShapeTransform = { _, _, bounds in commits += 1; annotation.bounds = bounds }
        view.onAnnotationSelection = { _, _ in
            view.frame.size.height -= 100
            view.scaleFactor = 0.8
            view.layoutDocumentView()
        }
        let point = view.convert(CGPoint(x: original.midX, y: original.midY), from: page)
        let down = event(.leftMouseDown, at: point, in: view)
        let up = event(.leftMouseUp, at: point, in: view)
        view.mouseDown(with: down); view.mouseUp(with: up)
        XCTAssertEqual(annotation.bounds, original)
        XCTAssertEqual(commits, 0, "Selecting a shape must not create an undo transaction")
        window.contentView = nil
    }

    func testNativeMouseMoveResizeBlankDeselectionAndContinuousStyle() throws {
        let c = WorkbenchController(); _ = c.view
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = c
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation); c.addRectangle(); c.view.layoutSubtreeIfNeeded()
        func gesture(_ start: CGPoint, _ end: CGPoint) throws {
            let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
            let a = c.pdfView.convert(start, from: page), z = c.pdfView.convert(end, from: page)
            c.pdfView.mouseDown(with: event(.leftMouseDown, at: a, in: c.pdfView))
            c.pdfView.mouseDragged(with: event(.leftMouseDragged, at: z, in: c.pdfView))
            c.pdfView.mouseUp(with: event(.leftMouseUp, at: z, in: c.pdfView))
        }
        try gesture(CGPoint(x:70,y:100), CGPoint(x:170,y:180))
        XCTAssertEqual(c.workspace.document?.page(at:0)?.annotations.count, 1)
        try gesture(CGPoint(x:120,y:140), CGPoint(x:150,y:160))
        var annotation = try XCTUnwrap(c.workspace.document?.page(at:0)?.annotations.first)
        XCTAssertEqual(annotation.bounds.minX, 100, accuracy:0.1)
        XCTAssertEqual(annotation.bounds.minY, 120, accuracy:0.1)
        try gesture(CGPoint(x:200,y:200), CGPoint(x:230,y:240))
        annotation = try XCTUnwrap(c.workspace.document?.page(at:0)?.annotations.first)
        XCTAssertEqual(annotation.bounds.width, 130, accuracy:0.1)
        XCTAssertEqual(annotation.bounds.height, 120, accuracy:0.1)
        c.changeShapeStyle(AnnotationStyle(strokeColor:.blue,fillColor:nil,lineWidth:5))
        try gesture(CGPoint(x:300,y:300),CGPoint(x:400,y:380))
        XCTAssertEqual(c.workspace.document?.page(at:0)?.annotations.count,2)
        XCTAssertEqual(c.workspace.document?.page(at:0)?.annotations.last?.border?.lineWidth,5)
        XCTAssertEqual(c.workspace.document?.page(at:0)?.annotations.first?.border?.lineWidth,5)
        c.enterMode(.drawing)
        XCTAssertFalse(c.pdfView.hasShapePreview)
        XCTAssertTrue(c.shapeStyleBar.isHidden)
        window.contentViewController = nil
    }

    func testVisibleOverlayRendersDifferentSizesBeforeMouseRelease() throws {
        let c = WorkbenchController(); _ = c.view
        let window = NSWindow(contentRect:NSRect(x:0,y:0,width:1280,height:820),styleMask:[.titled],backing:.buffered,defer:false)
        window.contentViewController=c
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation); c.addRectangle(); c.view.layoutSubtreeIfNeeded()
        let page=try XCTUnwrap(c.pdfView.document?.page(at:0))
        let visible = c.pdfView.convert(page.bounds(for:.cropBox),from:page).intersection(c.pdfView.bounds)
        XCTAssertFalse(visible.isEmpty)
        let start = c.pdfView.convert(CGPoint(x:visible.minX+70,y:visible.minY+80),to:page)
        let small = c.pdfView.convert(CGPoint(x:visible.minX+150,y:visible.minY+150),to:page)
        let large = c.pdfView.convert(CGPoint(x:visible.minX+300,y:visible.minY+250),to:page)
        c.pdfView.beginGesture(on:page,at:start)
        c.pdfView.updateGesture(to:small)
        let early=try XCTUnwrap(c.pdfView.renderShapeOverlay())
        c.pdfView.updateGesture(to:large)
        let later=try XCTUnwrap(c.pdfView.renderShapeOverlay())
        XCTAssertNotEqual(early,later,"Moving while pressed must change rendered pixels, not only stored coordinates")
        XCTAssertEqual(c.workspace.document?.page(at:0)?.annotations.count,0)
        let file=FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".png")
        defer { try? FileManager.default.removeItem(at:file) }
        try later.write(to:file)
        c.pdfView.cancelPlacementForEscape(); window.contentViewController=nil
    }

    func testAllFiveShapesPreviewAtZoomAndRotationAndCancelOnModeSwitch() throws {
        for rotation in [0,90,270] {
            let c=WorkbenchController(); _=c.view
            let window=NSWindow(contentRect:NSRect(x:0,y:0,width:1280,height:820),styleMask:[.titled],backing:.buffered,defer:false)
            window.contentViewController=c
            try c.workspace.createBlank(); try c.workspace.rotate(page:0,degrees:rotation)
            c.refreshDocument(); c.enterMode(.annotation); c.pdfView.scaleFactor=1.1; c.view.layoutSubtreeIfNeeded()
            for tool in ShapeToolKind.allCases {
                c.beginShapeTool(tool)
                let p=try XCTUnwrap(c.pdfView.document?.page(at:0))
                let visible=c.pdfView.convert(p.bounds(for:.cropBox),from:p).intersection(c.pdfView.bounds)
                let a=c.pdfView.convert(CGPoint(x:visible.minX+60,y:visible.minY+60),to:p)
                let z=c.pdfView.convert(CGPoint(x:visible.minX+220,y:visible.minY+150),to:p)
                let before=try XCTUnwrap(c.pdfView.renderShapeOverlay())
                c.pdfView.beginGesture(on:p,at:a); c.pdfView.updateGesture(to:z)
                XCTAssertNotEqual(before,try XCTUnwrap(c.pdfView.renderShapeOverlay()),"\(tool) at \(rotation)° must visibly render before release")
                XCTAssertEqual(c.workspace.document?.page(at:0)?.annotations.count,0)
                c.enterMode(.editing)
                XCTAssertFalse(c.pdfView.hasShapePreview)
                c.pdfView.finishGesture()
                XCTAssertEqual(c.workspace.document?.page(at:0)?.annotations.count,0)
                c.enterMode(.annotation)
            }
            window.contentViewController=nil
        }
    }
}
