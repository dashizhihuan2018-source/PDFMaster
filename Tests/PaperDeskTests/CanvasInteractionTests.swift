import XCTest
import AppKit
import PDFKit
import PaperCore
@testable import PaperDesk

@MainActor final class CanvasInteractionTests: XCTestCase {
    func testRectangleIsVisibleBeforeReleaseAndCommitIsUndoable() throws {
        let s = CanvasSession(); let v = CanvasView(session: s)
        v.frame = CGRect(x:0,y:0,width:800,height:600); v.tool = .rectangle
        let before = try XCTUnwrap(v.renderPNG())
        v.beginGesture(at: CGPoint(x:60,y:70)); v.updateGesture(to:CGPoint(x:180,y:160))
        XCTAssertEqual(s.board.elements.count,0)
        XCTAssertNotNil(v.previewElement)
        XCTAssertNotEqual(before,try XCTUnwrap(v.renderPNG()))
        v.finishGesture(); XCTAssertEqual(s.board.elements.count,1)
        s.undo(); XCTAssertTrue(s.board.elements.isEmpty)
    }
    func testInkUpdatesContinuouslyAndEscapeLeavesNoUndoEntry() throws {
        let s = CanvasSession(); let v = CanvasView(session:s); v.tool = .ink
        v.beginGesture(at:CGPoint(x:20,y:30)); v.updateGesture(to:CGPoint(x:40,y:55))
        let early = try XCTUnwrap(v.previewElement)
        v.updateGesture(to:CGPoint(x:70,y:42))
        XCTAssertGreaterThan(try XCTUnwrap(v.previewElement).points.count,early.points.count)
        v.cancelGesture(); XCTAssertNil(v.previewElement); XCTAssertFalse(s.canUndo)
    }
    func testMovePreviewDoesNotMutateModelAndHasSingleUndo() throws {
        let element = CanvasElement(kind:.rectangle,frame:CGRect(x:10,y:10,width:100,height:60))
        let s = CanvasSession(board:CanvasBoard(elements:[element])); let v = CanvasView(session:s)
        v.beginGesture(at:CGPoint(x:60,y:40)); v.updateGesture(to:CGPoint(x:90,y:65))
        XCTAssertEqual(s.board.elements[0].frame,element.frame)
        XCTAssertEqual(v.previewElement?.frame.minX,40)
        v.finishGesture(); XCTAssertEqual(s.board.elements[0].frame.minX,40)
        s.undo(); XCTAssertEqual(s.board.elements[0].frame,element.frame); XCTAssertFalse(s.canUndo)
    }
    func testResizeHasLivePreviewAndRetainsIdentity() throws {
        let element = CanvasElement(kind:.ellipse,frame:CGRect(x:20,y:20,width:80,height:60))
        let s = CanvasSession(board:CanvasBoard(elements:[element])); let v = CanvasView(session:s)
        v.select(element.id); v.beginGesture(at:CGPoint(x:100,y:80)); v.updateGesture(to:CGPoint(x:140,y:110))
        XCTAssertEqual(v.previewElement?.frame.width,120)
        v.finishGesture(); XCTAssertEqual(s.board.elements[0].id,element.id)
        XCTAssertEqual(s.board.elements[0].frame.height,90)
    }
    func testZoomKeepsAnchorStableAndHandPanDoesNotDirtyBoard() {
        let s = CanvasSession(); let v = CanvasView(session:s)
        let anchor = CGPoint(x:240,y:170); let world = v.viewport.worldPoint(from:anchor)
        v.zoom(to:2,anchor:anchor)
        XCTAssertEqual(v.viewport.viewPoint(from:world).x,anchor.x,accuracy:0.001)
        v.tool = .hand; v.beginGesture(at:.zero); v.updateGesture(to:CGPoint(x:80,y:40)); v.finishGesture()
        XCTAssertFalse(s.isDirty); XCTAssertFalse(s.canUndo)
    }
    func testControllerImportsPDFCardAndReopensEmbeddedData() throws {
        let c = CanvasController(); _ = c.view
        let doc = PDFDocument(); let page = PDFPage(); doc.insert(page,at:0)
        let data = try XCTUnwrap(doc.dataRepresentation())
        try c.importAsset(data:data,name:"测试.pdf",kind:.pdf)
        let item = try XCTUnwrap(c.session.board.elements.first)
        XCTAssertEqual(item.assetData,data)
        var received:Data?
        c.onOpenPDF = { content,_ in received = content }
        c.canvasView.activate(item.id)
        XCTAssertEqual(received,data)
    }
    func testTextEditingAndStyleChangesAreUndoableAndPersist() throws {
        let s = CanvasSession(); let v = CanvasView(session:s)
        v.tool = .sticky; v.beginGesture(at:CGPoint(x:40,y:60)); v.finishGesture()
        let id = try XCTUnwrap(s.board.elements.first?.id)
        v.commitText("会议记录",for:id)
        v.applyStyle(stroke:"#FF0000",fill:"#FFFF00",width:5)
        XCTAssertEqual(s.board.elements[0].text,"会议记录")
        XCTAssertEqual(s.board.elements[0].lineWidth,5)
        s.undo(); XCTAssertEqual(s.board.elements[0].lineWidth,2)
        XCTAssertEqual(try CanvasBoard.decode(s.board.encoded()).elements[0].text,"会议记录")
    }
    func testImportedInvalidPDFIsRejectedWithoutMutation() {
        let c = CanvasController()
        XCTAssertThrowsError(try c.importAsset(data:Data("not PDF".utf8),name:"坏.pdf",kind:.pdf))
        XCTAssertTrue(c.session.board.elements.isEmpty)
    }
    func testAllContentExportUsesBoundedDimensionsAndValidPDF() throws {
        let item = CanvasElement(kind:.sticky,frame:CGRect(x:-3000,y:1400,width:200,height:180),text:"画布导出")
        let v = CanvasView(session:CanvasSession(board:CanvasBoard(elements:[item])))
        v.frame = CGRect(x:0,y:0,width:800,height:600)
        let before = v.viewport
        let png = try XCTUnwrap(v.renderPNGData(allContent:true))
        XCTAssertNotNil(NSImage(data:png)); XCTAssertEqual(v.viewport,before)
        let pdf = try XCTUnwrap(PDFDocument(data:try XCTUnwrap(v.exportPDFData())))
        XCTAssertEqual(pdf.pageCount,1)
        XCTAssertGreaterThan(pdf.page(at:0)!.bounds(for:.mediaBox).width,0)
    }
    func testToolbarFitsCommonWindowWithoutScrolling() {
        let c = CanvasController(); c.view.frame = CGRect(x:0,y:0,width:1180,height:800)
        c.view.layoutSubtreeIfNeeded()
        XCTAssertGreaterThan(c.canvasView.bounds.width,1100)
        XCTAssertGreaterThan(c.canvasView.bounds.height,600)
    }
    func testNativeMouseEventsRenderInkBeforeMouseUpAtZoom() throws {
        let c = CanvasController(); c.view.frame = CGRect(x:0,y:0,width:1180,height:800)
        c.view.layoutSubtreeIfNeeded(); let v = c.canvasView; v.tool = .ink; v.zoom(to:2,anchor:.zero)
        func event(_ kind:NSEvent.EventType,_ point:CGPoint) -> NSEvent {
            NSEvent.mouseEvent(with:kind,location:v.convert(point,to:nil),modifierFlags:[],timestamp:0,windowNumber:0,context:nil,eventNumber:0,clickCount:1,pressure:1)!
        }
        v.mouseDown(with:event(.leftMouseDown,CGPoint(x:100,y:120)))
        v.mouseDragged(with:event(.leftMouseDragged,CGPoint(x:180,y:170)))
        let early = try XCTUnwrap(v.renderPNGData())
        v.mouseDragged(with:event(.leftMouseDragged,CGPoint(x:250,y:130)))
        XCTAssertNotEqual(early,try XCTUnwrap(v.renderPNGData()))
        XCTAssertTrue(c.session.board.elements.isEmpty)
        v.mouseUp(with:event(.leftMouseUp,CGPoint(x:250,y:130)))
        XCTAssertEqual(c.session.board.elements.count,1)
        XCTAssertEqual(c.session.board.elements[0].points.first?.x,50)
    }
    func testMalformedEmbeddedAssetsDoNotReplaceOpenBoard() throws {
        let c = CanvasController(session:CanvasSession(board:CanvasBoard(title:"保留我的画布")))
        let malformed = CanvasBoard(title:"坏附件",elements:[CanvasElement(kind:.image,frame:CGRect(x:0,y:0,width:100,height:100),assetData:Data("fake image".utf8),assetName:"x.png")])
        XCTAssertThrowsError(try CanvasController.validatedAssets(malformed))
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("canvas-invalid-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        defer { try? FileManager.default.removeItem(at:directory) }
        let file = directory.appendingPathComponent("bad.pdfmasterboard"); try malformed.save(to:file)
        XCTAssertThrowsError(try c.openBoard(url:file))
        XCTAssertEqual(c.session.board.title,"保留我的画布"); XCTAssertNil(c.sourceURL)
    }
    func testStyleFromSelectedObjectMatchesNextStrokeAndLargeExportKeepsView() throws {
        let item = CanvasElement(kind:.rectangle,frame:CGRect(x:0,y:0,width:200,height:200),strokeHex:"#123456",lineWidth:6)
        let c = CanvasController(session:CanvasSession(board:CanvasBoard(elements:[item])))
        _ = c.view; c.canvasView.select(item.id)
        c.canvasView.tool = .ink; c.canvasView.beginGesture(at:CGPoint(x:10,y:20)); c.canvasView.updateGesture(to:CGPoint(x:80,y:90))
        XCTAssertEqual(c.canvasView.previewElement?.strokeHex,"#123456"); XCTAssertEqual(c.canvasView.previewElement?.lineWidth,6)
        c.canvasView.cancelGesture()
        try c.session.mutate { $0.elements.append(CanvasElement(kind:.rectangle,frame:CGRect(x:100_000_000,y:0,width:200,height:200))) }
        let before = c.canvasView.viewport, frame = c.canvasView.frame
        let data = try XCTUnwrap(c.canvasView.renderPNGData(allContent:true))
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data:data))
        XCTAssertEqual(bitmap.pixelsWide,4096)
        XCTAssertEqual(c.canvasView.viewport,before); XCTAssertEqual(c.canvasView.frame,frame)
    }
    func testPDFCardThumbnailActuallyContainsDocumentPixels() throws {
        let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:32,pixelsHigh:32,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:128,bitsPerPixel:32))
        let sourceContext = try XCTUnwrap(NSGraphicsContext(bitmapImageRep:bitmap)?.cgContext)
        sourceContext.setFillColor(CGColor(red:1,green:0,blue:0,alpha:1))
        sourceContext.fill(CGRect(x:0,y:0,width:32,height:32))
        let image = NSImage(cgImage:try XCTUnwrap(bitmap.cgImage),size:CGSize(width:32,height:32))
        let document = PDFDocument(); document.insert(try XCTUnwrap(PDFPage(image:image)),at:0)
        let c = CanvasController(); c.view.frame = CGRect(x:0,y:0,width:1180,height:800); c.view.layoutSubtreeIfNeeded()
        try c.importAsset(data:try XCTUnwrap(document.dataRepresentation()),name:"红色测试.pdf",kind:.pdf)
        c.canvasView.fitContent()
        let decoded = try XCTUnwrap(NSBitmapImageRep(data:try XCTUnwrap(c.canvasView.renderPNGData())))
        let sourceImage = try XCTUnwrap(decoded.cgImage)
        let rendered = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:sourceImage.width,pixelsHigh:sourceImage.height,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:sourceImage.width * 4,bitsPerPixel:32))
        let context = try XCTUnwrap(NSGraphicsContext(bitmapImageRep:rendered)?.cgContext)
        context.draw(sourceImage,in:CGRect(x:0,y:0,width:sourceImage.width,height:sourceImage.height))
        var redPixels = 0
        for x in stride(from:0,to:rendered.pixelsWide,by:10) { for y in stride(from:0,to:rendered.pixelsHigh,by:10) {
            if let color = rendered.colorAt(x:x,y:y)?.usingColorSpace(.sRGB),color.redComponent > 0.8,color.greenComponent < 0.2 { redPixels += 1 }
        } }
        XCTAssertGreaterThan(redPixels,100,"PDF card must show rendered source content, not just a white placeholder")
    }
}
