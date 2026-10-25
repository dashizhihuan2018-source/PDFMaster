import XCTest
import AppKit
import PDFKit
import PaperCore
@testable import PaperDesk

@MainActor final class InkInteractionTests: XCTestCase {
    func testInkPreviewRemainsVisibleUntilCommitCallbackFinishes() throws {
        let view = InkPDFView(frame: CGRect(x: 0, y: 0, width: 640, height: 720))
        let document = PDFDocument()
        let page = PDFPage()
        page.setBounds(CGRect(x: 0, y: 0, width: 500, height: 700), for: .mediaBox)
        document.insert(page, at: 0)
        view.document = document; view.inkEnabled = true
        view.beginGesture(on: page, at: CGPoint(x: 70, y: 100))
        view.updateGesture(to: CGPoint(x: 170, y: 180))
        let overlay = try XCTUnwrap(view.subviews.first { $0.layer?.zPosition == 1_000 })
        let previewLayer = try XCTUnwrap(overlay.layer?.sublayers?.first as? CAShapeLayer)
        let path = try XCTUnwrap(view.inkPreviewPath)
        var commitCount = 0
        view.onInk = { _, _, _ in
            commitCount += 1
            XCTAssertTrue(view.hasInkPreview, "Do not flush an empty preview before committing the persistent annotation")
            XCTAssertFalse(previewLayer.isHidden)
            XCTAssertTrue(view.inkPreviewPath === path)
        }
        view.finishGesture()
        XCTAssertEqual(commitCount, 1)
        XCTAssertFalse(view.hasInkPreview)
        XCTAssertTrue(previewLayer.isHidden)
        view.finishGesture()
        XCTAssertEqual(commitCount, 1)
    }

    func testEachInkCommitPreservesPresentationIdentityAndViewport() throws {
        let c = WorkbenchController(); _ = c.view
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = c
        defer { window.contentViewController = nil }
        try c.workspace.createBlank(); try c.workspace.insertBlank(at: 1)
        c.refreshDocument(); c.enterMode(.annotation); c.toggleInk(NSMenuItem())
        try c.readerSession.setZoom(.percent(175))
        c.view.layoutSubtreeIfNeeded(); c.pdfView.layoutDocumentView()
        let document = try XCTUnwrap(c.pdfView.document)
        let page = try XCTUnwrap(document.page(at: 1))
        c.pdfView.go(to: CGRect(x: 70, y: 200, width: 100, height: 100), on: page)
        let documentView = try XCTUnwrap(c.pdfView.documentView)
        let clip = try XCTUnwrap(documentView.enclosingScrollView?.contentView)
        let origin = clip.bounds.origin
        let scale = c.pdfView.scaleFactor
        let revision = c.workspace.revision
        for stroke in 0..<3 {
            c.pdfView.beginGesture(on: page, at: CGPoint(x: 70, y: 250 + stroke * 30))
            c.pdfView.updateGesture(to: CGPoint(x: 170, y: 270 + stroke * 30))
            c.pdfView.finishGesture()
            XCTAssertTrue(c.pdfView.document === document, "A local ink commit must not replace PDFView.document")
            XCTAssertTrue(c.pdfView.document?.page(at: 1) === page)
            XCTAssertTrue(c.pdfView.documentView === documentView)
            XCTAssertTrue(c.pdfView.documentView?.enclosingScrollView?.contentView === clip)
            XCTAssertEqual(clip.bounds.origin.x, origin.x, accuracy: 0.001)
            XCTAssertEqual(clip.bounds.origin.y, origin.y, accuracy: 0.001)
            XCTAssertEqual(c.pdfView.scaleFactor, scale, accuracy: 0.0001)
            XCTAssertEqual(c.readerSession.currentZoom, .percent(175))
            XCTAssertEqual(c.workspace.revision, revision + stroke + 1)
            XCTAssertEqual(c.pdfView.document?.page(at: 1)?.annotations.count, stroke + 1)
            XCTAssertEqual(c.workspace.document?.page(at: 1)?.annotations.count, stroke + 1)
        }
        c.undo()
        XCTAssertEqual(c.workspace.document?.page(at: 1)?.annotations.count, 2)
        c.redo()
        XCTAssertEqual(c.workspace.document?.page(at: 1)?.annotations.count, 3)
    }

    private func event(_ type: NSEvent.EventType, pagePoint: CGPoint, page: PDFPage, view: InkPDFView) -> NSEvent {
        let point = view.convert(view.convert(pagePoint, from: page), to: nil)
        return NSEvent.mouseEvent(with: type, location: point, modifierFlags: [], timestamp: 0,
            windowNumber: view.window?.windowNumber ?? 0, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
    }

    func testNativeDragChangesVisibleInkBeforeReleaseAndCommitsOnlyOnce() throws {
        let c = WorkbenchController(); _ = c.view
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = c
        window.setFrameOrigin(NSPoint(x: -10_000, y: -10_000))
        window.orderFront(nil)
        defer { window.orderOut(nil); window.contentViewController = nil }
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation); c.toggleInk(NSMenuItem())
        c.view.layoutSubtreeIfNeeded()
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        let visible = c.pdfView.convert(page.bounds(for: .cropBox), from: page).intersection(c.pdfView.bounds)
        func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint { c.pdfView.convert(CGPoint(x: visible.minX + x, y: visible.minY + y), to: page) }
        let start = point(50, 60), middle = point(140, 130), end = point(230, 80)
        let before = try XCTUnwrap(c.pdfView.renderShapeOverlay())
        let revision = c.workspace.revision
        c.pdfView.mouseDown(with: event(.leftMouseDown, pagePoint: start, page: page, view: c.pdfView))
        c.pdfView.mouseDragged(with: event(.leftMouseDragged, pagePoint: middle, page: page, view: c.pdfView))
        let overlay = try XCTUnwrap(c.pdfView.subviews.first { $0.layer?.zPosition == 1_000 })
        let previewLayer = try XCTUnwrap(overlay.layer?.sublayers?.first as? CAShapeLayer)
        XCTAssertNotNil(previewLayer.path, "The drag must update the model-layer path")
        XCTAssertNotNil(previewLayer.presentation()?.path,
                        "The drag must flush the path to the compositor before mouseUp")
        let early = try XCTUnwrap(c.pdfView.renderShapeOverlay())
        XCTAssertNotEqual(before, early, "The actual overlay must paint during drag, not just retain points")
        c.pdfView.mouseDragged(with: event(.leftMouseDragged, pagePoint: end, page: page, view: c.pdfView))
        let late = try XCTUnwrap(c.pdfView.renderShapeOverlay())
        XCTAssertNotEqual(early, late)
        let artifact = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("docs/verification/ink-drag-in-progress.png")
        try late.write(to: artifact)
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        c.pdfView.mouseUp(with: event(.leftMouseUp, pagePoint: end, page: page, view: c.pdfView))
        XCTAssertEqual(c.workspace.revision, revision + 1)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.type, "Ink")
        window.orderOut(nil)
        c.undo()
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
    }

    func testEscapeExitsPenAndCancelsWithoutDocumentMutation() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation); c.toggleInk(NSMenuItem())
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        c.pdfView.beginGesture(on: page, at: CGPoint(x: 70, y: 100))
        c.pdfView.updateGesture(to: CGPoint(x: 120, y: 180))
        let revision = c.workspace.revision
        XCTAssertTrue(c.pdfView.cancelPlacementForEscape())
        XCTAssertFalse(c.pdfView.inkEnabled)
        c.pdfView.finishGesture()
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
    }

    func testZoomRotationCropMappingAndExactPreviewPathSurviveSaveReopen() throws {
        for rotation in [0,90,180,270] {
            let c = WorkbenchController(); _ = c.view
            let window = NSWindow(contentRect: NSRect(x:0,y:0,width:1280,height:820), styleMask:[.titled], backing:.buffered, defer:false)
            window.contentViewController = c
            defer { window.contentViewController = nil }
            try c.workspace.createBlank()
            let crop = CGRect(x:37,y:53,width:400,height:500)
            try c.workspace.cropPage(at:0,to:crop); try c.workspace.rotate(page:0,degrees:rotation)
            c.refreshDocument(); c.enterMode(.annotation); c.toggleInk(NSMenuItem())
            c.changeInkStyle(AnnotationStyle(strokeColor:.blue,fillColor:nil,lineWidth:8))
            c.pdfView.scaleFactor = 1.1; c.view.layoutSubtreeIfNeeded()
            let page = try XCTUnwrap(c.pdfView.document?.page(at:0))
            let points = [CGPoint(x:90,y:140),CGPoint(x:140,y:200),CGPoint(x:200,y:110),CGPoint(x:280,y:260)]
            let before = try XCTUnwrap(c.pdfView.renderShapeOverlay())
            c.pdfView.mouseDown(with:event(.leftMouseDown,pagePoint:points[0],page:page,view:c.pdfView))
            for point in points.dropFirst() { c.pdfView.mouseDragged(with:event(.leftMouseDragged,pagePoint:point,page:page,view:c.pdfView)) }
            XCTAssertTrue(c.pdfView.hasInkPreview)
            XCTAssertNotEqual(before,try XCTUnwrap(c.pdfView.renderShapeOverlay()), "Visible ink at \(rotation)°")
            let preview = try XCTUnwrap(c.pdfView.inkPreviewPath)
            // Changing settings mid-stroke must not restyle the displayed or committed stroke.
            c.changeInkStyle(AnnotationStyle(strokeColor:.red,fillColor:nil,lineWidth:2))
            c.pdfView.mouseUp(with:event(.leftMouseUp,pagePoint:points.last!,page:page,view:c.pdfView))
            XCTAssertFalse(c.pdfView.hasInkPreview)
            let annotation = try XCTUnwrap(c.workspace.document?.page(at:0)?.annotations.first)
            XCTAssertEqual(annotation.color.usingColorSpace(.deviceRGB),NSColor.blue.usingColorSpace(.deviceRGB))
            XCTAssertEqual(annotation.border?.lineWidth,8)
            let local = try XCTUnwrap(annotation.paths?.first)
            var offset = CGAffineTransform(translationX:-annotation.bounds.minX,y:-annotation.bounds.minY)
            let expected = InkStrokePath.bezier(try XCTUnwrap(preview.copy(using:&offset)))
            assertSamePath(local,expected)
            let saved = FileManager.default.temporaryDirectory.appendingPathComponent("pdfmaster-ink-\(UUID().uuidString).pdf")
            defer { try? FileManager.default.removeItem(at:saved) }
            try c.workspace.saveCopy(to:saved)
            let reopened = try XCTUnwrap(PDFDocument(url:saved))
            let reopenedAnnotation = try XCTUnwrap(reopened.page(at:0)?.annotations.first)
            XCTAssertEqual(reopenedAnnotation.type,"Ink")
            XCTAssertEqual(reopenedAnnotation.border?.lineWidth,8)
            assertSamePath(try XCTUnwrap(reopenedAnnotation.paths?.first),expected,accuracy:0.02)
        }
    }

    func testCropClipsWideInkAndModeDocumentAndNoMovementCancel() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation); c.toggleInk(NSMenuItem())
        c.changeInkStyle(AnnotationStyle(strokeColor:.black,fillColor:nil,lineWidth:12))
        let page = try XCTUnwrap(c.pdfView.document?.page(at:0)), crop = page.bounds(for:.cropBox)
        c.pdfView.beginGesture(on:page,at:CGPoint(x:-100,y:-100))
        c.pdfView.updateGesture(to:CGPoint(x:2000,y:2000))
        let path = try XCTUnwrap(c.pdfView.inkPreviewPath)
        XCTAssertTrue(crop.contains(path.boundingBoxOfPath.insetBy(dx:-6,dy:-6)))
        let revision = c.workspace.revision
        c.enterMode(.editing); c.pdfView.finishGesture()
        XCTAssertFalse(c.pdfView.hasInkPreview); XCTAssertEqual(c.workspace.revision,revision)
        c.enterMode(.annotation); c.toggleInk(NSMenuItem())
        c.pdfView.beginGesture(on:page,at:CGPoint(x:70,y:100)); c.pdfView.updateGesture(to:CGPoint(x:170,y:180))
        c.refreshDocument(); c.pdfView.finishGesture()
        XCTAssertFalse(c.pdfView.hasInkPreview); XCTAssertEqual(c.workspace.revision,revision)
        let next = try XCTUnwrap(c.pdfView.document?.page(at:0))
        c.pdfView.beginGesture(on:next,at:CGPoint(x:70,y:100)); c.pdfView.updateGesture(to:CGPoint(x:70,y:100)); c.pdfView.finishGesture()
        XCTAssertEqual(c.workspace.revision,revision)
    }

    private func assertSamePath(_ a: NSBezierPath, _ b: NSBezierPath, accuracy: CGFloat = 0.0001, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(a.elementCount,b.elementCount,file:file,line:line)
        guard a.elementCount == b.elementCount else { return }
        for i in 0..<a.elementCount {
            var x=[CGPoint](repeating:.zero,count:3), y=x
            let tx=a.element(at:i,associatedPoints:&x), ty=b.element(at:i,associatedPoints:&y)
            XCTAssertEqual(tx,ty,file:file,line:line)
            let count = tx == .cubicCurveTo ? 3 : (tx == .closePath ? 0 : 1)
            for j in 0..<count { XCTAssertEqual(x[j].x,y[j].x,accuracy:accuracy,file:file,line:line); XCTAssertEqual(x[j].y,y[j].y,accuracy:accuracy,file:file,line:line) }
        }
    }
}
