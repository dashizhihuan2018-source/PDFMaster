import XCTest
import AppKit
import PDFKit
import PaperCore
@testable import PaperDesk

@MainActor final class ContentPlacementTests: XCTestCase {
    func testTextClickUsesReadableDefaultSizeAtTheClickAndClampsToCrop() throws {
        let tool = PendingPlacement.freeText("跟随点击的文字")
        let crop = CGRect(x: 31, y: 57, width: 600, height: 800)
        let center = CGPoint(x: 198, y: 589)
        let bounds = try tool.bounds(from: center, to: center, within: crop)
        XCTAssertGreaterThanOrEqual(bounds.width, 160)
        XCTAssertGreaterThanOrEqual(bounds.height, 36)
        XCTAssertEqual(bounds.midX, center.x, accuracy: 0.01)
        XCTAssertEqual(bounds.midY, center.y, accuracy: 0.01)
        for point in [crop.origin, CGPoint(x: crop.maxX, y: crop.maxY)] {
            let edgeBounds = try tool.bounds(from: point, to: point, within: crop)
            XCTAssertTrue(crop.contains(edgeBounds))
            XCTAssertEqual(edgeBounds.size, bounds.size, "Clamp location instead of clipping text at the page edge")
        }
    }

    func testNoteClickAndDragBoundsRespectNonzeroCropOrigin() throws {
        let tool = PendingPlacement.note(NoteConfiguration(contents: "便笺", author: nil, color: .yellow))
        let crop = CGRect(x: 17, y: 39, width: 500, height: 700)
        let point = CGPoint(x: 160, y: 300)
        let bounds = try tool.bounds(from: point, to: point, within: crop)
        XCTAssertEqual(bounds.size, CGSize(width: 24, height: 24))
        XCTAssertEqual(bounds.midX, point.x, accuracy: 0.01)
        XCTAssertEqual(bounds.midY, point.y, accuracy: 0.01)
        XCTAssertEqual(try tool.bounds(from: CGPoint(x: 230, y: 390), to: CGPoint(x: 90, y: 270), within: crop), CGRect(x: 90, y: 270, width: 140, height: 120))
    }

    func testClickDeliversPlacementWithoutRequiringADrag() {
        let view = InkPDFView(frame: NSRect(x: 0, y: 0, width: 800, height: 650))
        let document = PDFDocument(), page = PDFPage()
        page.setBounds(CGRect(x: 0, y: 0, width: 600, height: 800), for: .mediaBox)
        document.insert(page, at: 0); view.document = document; view.placementEnabled = true
        let click = CGPoint(x: 97, y: 523)
        var placements: [(CGPoint, CGPoint, Int)] = []
        view.onPlacement = { placements.append(($0, $1, $2)) }
        view.beginGesture(on: page, at: click); view.finishGesture()
        XCTAssertEqual(placements.count, 1, "Content placement must accept a click, not require a rectangle")
        XCTAssertEqual(placements.first?.0, click)
        XCTAssertEqual(placements.first?.1, click)
        XCTAssertEqual(placements.first?.2, 0)
    }

    func testPendingContentClickWinsOverExistingAnnotationSelection() throws {
        let view = InkPDFView(frame: NSRect(x: 0, y: 0, width: 800, height: 650))
        let window = NSWindow(contentRect: view.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = view
        defer { window.contentView = nil }
        let document = PDFDocument(), page = PDFPage()
        page.setBounds(CGRect(x: 0, y: 0, width: 600, height: 800), for: .mediaBox)
        document.insert(page, at: 0); view.document = document; view.scaleFactor = 0.7; view.layoutDocumentView()
        let target = CGPoint(x: 180, y: 420)
        page.addAnnotation(PDFAnnotation(bounds: CGRect(x: 150, y: 390, width: 60, height: 60), forType: .square, withProperties: nil))
        view.placementEnabled = true
        var placed: CGPoint?, selections = 0
        view.onPlacement = { start, _, _ in placed = start }
        view.onAnnotationSelection = { _, _ in selections += 1 }
        let point = view.convert(target, from: page)
        view.mouseDown(with: event(.leftMouseDown, at: point, in: view))
        view.mouseUp(with: event(.leftMouseUp, at: point, in: view))
        XCTAssertEqual(selections, 0, "An armed insertion tool must not be swallowed by annotation hit testing")
        XCTAssertEqual(try XCTUnwrap(placed).x, target.x, accuracy: 0.1)
    }

    func testTextAndNoteClicksCommitOnceOnClickedPageAndRemainUndoable() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); try c.workspace.insertBlank(at: 1); c.refreshDocument()
        let tools: [PendingPlacement] = [.freeText("跟随点击"), .note(NoteConfiguration(contents: "注释", author: "Nankong", color: .yellow))]
        for tool in tools {
            c.beginPlacement(tool, accessibilityName: "放置测试")
            XCTAssertTrue(c.interactionStatusText.contains("点击"))
            let page = try XCTUnwrap(c.pdfView.document?.page(at: 1))
            let revision = c.workspace.revision, click = CGPoint(x: 190, y: 527)
            c.pdfView.beginGesture(on: page, at: click)
            XCTAssertEqual(c.workspace.revision, revision)
            c.pdfView.finishGesture()
            XCTAssertEqual(c.workspace.revision, revision + 1)
            XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
            let bounds = try XCTUnwrap(c.workspace.document?.page(at: 1)?.annotations.last?.bounds)
            XCTAssertEqual(bounds.midX, click.x, accuracy: 0.1)
            XCTAssertEqual(bounds.midY, click.y, accuracy: 0.1)
            XCTAssertFalse(c.pdfView.placementEnabled, "Content insertion is one-shot")
            c.pdfView.finishGesture()
            XCTAssertEqual(c.workspace.revision, revision + 1)
            c.undo()
            XCTAssertEqual(c.workspace.document?.page(at: 1)?.annotations.count, 0)
        }
    }

    func testSignatureWaitsForClickUsesTargetPageAndPreservesSeparateStrokes() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); try c.workspace.insertBlank(at: 1); c.refreshDocument()
        let strokes = [[CGPoint(x: 0, y: 0), CGPoint(x: 1, y: 1)], [CGPoint(x: 0, y: 1), CGPoint(x: 1, y: 0)]]
        let revision = c.workspace.revision
        c.beginPlacement(.signature(strokes), accessibilityName: "签名")
        XCTAssertEqual(c.workspace.revision, revision, "Finishing capture must arm placement, not insert at page center")
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 1)), click = CGPoint(x: 150, y: 560)
        c.pdfView.beginGesture(on: page, at: click); c.pdfView.finishGesture()
        XCTAssertEqual(c.workspace.revision, revision + 1, "All signature strokes form one undo transaction")
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        let annotations = try XCTUnwrap(c.workspace.document?.page(at: 1)?.annotations)
        XCTAssertEqual(annotations.count, 2)
        let bounds = annotations.reduce(CGRect.null) { $0.union($1.bounds) }
        XCTAssertEqual(bounds.midX, click.x, accuracy: 0.1)
        XCTAssertEqual(bounds.midY, click.y, accuracy: 0.1)
        XCTAssertEqual(bounds.width, 164.4, accuracy: 0.1)
        c.undo()
        XCTAssertEqual(c.workspace.document?.page(at: 1)?.annotations.count, 0)
    }

    func testSignatureDragUsesChosenSizeAndAllRotationsStayInsideCropAtEdges() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank()
        let strokes = [[CGPoint(x: 0, y: 0), CGPoint(x: 1, y: 1)]]
        for rotation in [0, 90, 180, 270] {
            if rotation != 0 { try c.workspace.rotate(page: 0, degrees: 90) }
            c.refreshDocument()
            let page = try XCTUnwrap(c.pdfView.document?.page(at: 0)), crop = page.bounds(for: .cropBox)
            c.beginPlacement(.signature(strokes), accessibilityName: "签名")
            c.pdfView.beginGesture(on: page, at: CGPoint(x: crop.maxX, y: crop.maxY)); c.pdfView.finishGesture()
            let edge = try XCTUnwrap(c.workspace.document?.page(at: 0)?.annotations.last?.bounds)
            XCTAssertGreaterThanOrEqual(edge.minX, crop.minX-0.01)
            XCTAssertGreaterThanOrEqual(edge.minY, crop.minY-0.01)
            XCTAssertLessThanOrEqual(edge.maxX, crop.maxX+0.01)
            XCTAssertLessThanOrEqual(edge.maxY, crop.maxY+0.01)
            c.beginPlacement(.signature(strokes), accessibilityName: "签名")
            c.pdfView.beginGesture(on: page, at: CGPoint(x: 80, y: 200))
            c.pdfView.updateGesture(to: CGPoint(x: 380, y: 340)); c.pdfView.finishGesture()
            let dragged = try XCTUnwrap(c.workspace.document?.page(at: 0)?.annotations.last?.bounds)
            XCTAssertEqual(dragged.width, 304.4, accuracy: 0.1)
            XCTAssertEqual(dragged.height, 144.4, accuracy: 0.1)
        }
    }

    func testEscapeAndToolSwitchDiscardPendingContentWithoutUndo() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); c.refreshDocument()
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0)), revision = c.workspace.revision
        for tool in [PendingPlacement.freeText("取消"), .signature([[.zero, CGPoint(x: 1, y: 1)]])] {
            c.beginPlacement(tool, accessibilityName: "取消测试")
            c.pdfView.beginGesture(on: page, at: CGPoint(x: 100, y: 200))
            c.pdfView.updateGesture(to: CGPoint(x: 300, y: 400))
            XCTAssertTrue(c.pdfView.cancelPlacementForEscape())
            c.pdfView.finishGesture()
            XCTAssertEqual(c.workspace.revision, revision)
            XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
            XCTAssertFalse(c.pdfView.placementEnabled)
        }
        c.beginPlacement(.freeText("切换"), accessibilityName: "切换测试")
        c.pdfView.beginGesture(on: page, at: CGPoint(x: 100, y: 200))
        c.enterMode(.drawing); c.pdfView.finishGesture()
        XCTAssertEqual(c.workspace.revision, revision)
    }

    func testNativeClicksKeepPageCoordinatesAfterZoomRotationAndScroll() throws {
        let c = WorkbenchController(); _ = c.view
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = c
        defer { window.contentViewController = nil }
        try c.workspace.createBlank(); try c.workspace.insertBlank(at: 1)
        for rotation in [0, 90, 180, 270] {
            if rotation != 0 { try c.workspace.rotate(page: 1, degrees: 90) }
            c.refreshDocument(); c.pdfView.scaleFactor = 1.35
            let page = try XCTUnwrap(c.pdfView.document?.page(at: 1))
            c.pdfView.go(to: page)
            c.pdfView.documentView?.enclosingScrollView?.contentView.scroll(to: CGPoint(x: 40, y: 270))
            c.beginPlacement(.freeText("坐标"), accessibilityName: "定位")
            c.view.layoutSubtreeIfNeeded(); c.pdfView.layoutDocumentView()
            let visible = c.pdfView.convert(page.bounds(for: .cropBox), from: page).intersection(c.pdfView.bounds)
            XCTAssertGreaterThan(visible.width, 80)
            let point = CGPoint(x: visible.midX, y: visible.midY)
            let expected = c.pdfView.convert(point, to: page)
            c.pdfView.mouseDown(with: event(.leftMouseDown, at: point, in: c.pdfView))
            c.pdfView.mouseUp(with: event(.leftMouseUp, at: point, in: c.pdfView))
            let annotation = try XCTUnwrap(c.workspace.document?.page(at: 1)?.annotations.last)
            let expectedBounds = try PendingPlacement.freeText("坐标").bounds(from: expected, to: expected, within: page.bounds(for: .cropBox))
            XCTAssertEqual(annotation.bounds.midX, expectedBounds.midX, accuracy: 0.2, "\(rotation)° x")
            XCTAssertEqual(annotation.bounds.midY, expectedBounds.midY, accuracy: 0.2, "\(rotation)° y")
        }
    }

    func testSubFourScreenPointJitterIsAClickAtEveryZoom() throws {
        let view = InkPDFView(frame: NSRect(x: 0, y: 0, width: 800, height: 650))
        let document = PDFDocument(), page = PDFPage()
        page.setBounds(CGRect(x: 0, y: 0, width: 600, height: 800), for: .mediaBox)
        document.insert(page, at: 0); view.document = document; view.placementEnabled = true
        let click = CGPoint(x: 180, y: 320)
        for scale in [0.5, 1.0, 2.0] {
            view.scaleFactor = scale
            var end: CGPoint?
            view.onPlacement = { _, z, _ in end = z }
            view.beginGesture(on: page, at: click)
            view.updateGesture(to: CGPoint(x: click.x+2/scale, y: click.y)); view.finishGesture()
            XCTAssertEqual(end, click)
        }
    }

    func testSignatureStrokesRemainVisuallyHorizontalThroughAllPageRotations() {
        let view = InkPDFView(frame: NSRect(x: 0, y: 0, width: 800, height: 650))
        let document = PDFDocument(), page = PDFPage()
        page.setBounds(CGRect(x: 0, y: 0, width: 600, height: 800), for: .mediaBox)
        document.insert(page, at: 0); view.document = document
        let strokes = [[CGPoint(x: 0, y: 0.5), CGPoint(x: 1, y: 0.5)]]
        for rotation in [0, 90, 180, 270] {
            page.rotation = rotation; view.layoutDocumentView()
            let mapped = PendingPlacement.signatureStrokes(strokes, in: CGRect(x: 80, y: 200, width: 160, height: 70), rotation: rotation)[0]
            let a = view.convert(mapped[0], from: page), z = view.convert(mapped[1], from: page)
            XCTAssertGreaterThan(z.x, a.x, "\(rotation)° must preserve signature writing direction")
            XCTAssertEqual(z.y, a.y, accuracy: 0.01)
        }
    }

    func testDefaultSizeFitsTinyCropAndRejectsNonfiniteInput() throws {
        let tool = PendingPlacement.freeText("小页")
        let crop = CGRect(x: 37, y: 61, width: 90, height: 20), click = CGPoint(x: 50, y: 70)
        let bounds = try tool.bounds(from: click, to: click, within: crop)
        XCTAssertTrue(crop.contains(bounds))
        XCTAssertEqual(bounds.width / bounds.height, 240 / 72, accuracy: 0.01)
        XCTAssertThrowsError(try tool.bounds(from: CGPoint(x: CGFloat.infinity, y: 1), to: click, within: crop))
        XCTAssertThrowsError(try tool.bounds(from: click, to: click, within: .zero))
    }

    private func event(_ kind: NSEvent.EventType, at point: CGPoint, in view: InkPDFView) -> NSEvent {
        NSEvent.mouseEvent(with: kind, location: view.convert(point, to: nil), modifierFlags: [], timestamp: 0, windowNumber: view.window?.windowNumber ?? 0, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
    }
}
