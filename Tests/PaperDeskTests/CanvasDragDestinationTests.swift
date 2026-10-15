import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

/// Only NSDraggingInfo payload is synthetic. Destination handlers, NSPasteboard,
/// URL admission, PDF page preparation, model transaction and viewport are real.
/// This is not Finder-to-App/OS drag E2E.
@MainActor private final class CanvasDragPayload: NSObject, NSDraggingInfo {
    var draggingDestinationWindow: NSWindow?
    var draggingSourceOperationMask: NSDragOperation = .copy
    var draggingLocation: NSPoint = .zero
    var draggedImageLocation: NSPoint = .zero
    nonisolated var draggedImage: NSImage? { nil }
    let draggingPasteboard: NSPasteboard
    var draggingSource: Any? { nil }
    var draggingSequenceNumber: Int { 1 }
    var draggingFormation: NSDraggingFormation = .none
    var animatesToDestination = false
    var numberOfValidItemsForDrop = 0
    var springLoadingHighlight: NSSpringLoadingHighlight { .none }
    init(urls: [URL]) {
        draggingPasteboard = NSPasteboard(name: .init("CanvasDrag-\(UUID())"))
        super.init(); draggingPasteboard.writeObjects(urls.map { $0 as NSURL })
    }
    func slideDraggedImage(to screenPoint: NSPoint) {}
    nonisolated override func namesOfPromisedFilesDropped(atDestination dropDestination: URL) -> [String]? { nil }
    func resetSpringLoading() {}
    func enumerateDraggingItems(options: NSDraggingItemEnumerationOptions, for view: NSView?, classes: [AnyClass],
        searchOptions: [NSPasteboard.ReadingOptionKey: Any], using block: (NSDraggingItem, Int, UnsafeMutablePointer<ObjCBool>) -> Void) {}
}

@MainActor final class CanvasDragDestinationTests: XCTestCase {
    private func fixture() throws -> URL {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("CanvasDrop-\(UUID())")
        try FileManager.default.createDirectory(at:dir,withIntermediateDirectories:true)
        addTeardownBlock { try? FileManager.default.removeItem(at:dir) }
        let document = PDFDocument()
        for index in 0..<3 {
            let page = PDFPage(); page.setBounds(CGRect(x:0,y:0,width:300,height:400),for:.mediaBox)
            let annotation = PDFAnnotation(bounds:CGRect(x:20,y:20,width:30,height:30),forType:.text,withProperties:nil)
            annotation.contents = "Drop page \(index+1)"; page.addAnnotation(annotation); document.insert(page,at:index)
        }
        let url = dir.appendingPathComponent("fixture.pdf")
        try XCTUnwrap(document.dataRepresentation()).write(to:url); return url
    }
    func testDropHandlersImportSelectedPDFPageAtZoomedWorldPoint() throws {
        let url = try fixture(), original = try Data(contentsOf:url), c = CanvasController()
        c.view.frame = CGRect(x:0,y:0,width:900,height:650); c.view.layoutSubtreeIfNeeded()
        let v = c.canvasView; v.zoom(to:2,anchor:.zero); v.pan(by:CGSize(width:40,height:-20))
        c.pdfPageSelectionPrompt = { _,count in XCTAssertEqual(count,3); return .page(2) }
        let payload = CanvasDragPayload(urls:[url]); defer { payload.draggingPasteboard.releaseGlobally() }
        let local = CGPoint(x:340,y:260), world = v.viewport.worldPoint(from:local)
        payload.draggingLocation = v.convert(local,to:nil)
        XCTAssertEqual(v.draggingEntered(payload),.copy); XCTAssertTrue(v.prepareForDragOperation(payload))
        XCTAssertTrue(v.performDragOperation(payload)); v.concludeDragOperation(payload)
        let item = try XCTUnwrap(c.session.board.elements.first)
        XCTAssertEqual(item.frame.midX,world.x); XCTAssertEqual(item.frame.midY,world.y)
        let embedded = try XCTUnwrap(PDFDocument(data:try XCTUnwrap(item.assetData)))
        XCTAssertEqual(embedded.pageCount,1)
        XCTAssertTrue(try XCTUnwrap(embedded.page(at:0)).annotations.contains { $0.contents == "Drop page 2" })
        XCTAssertEqual(try Data(contentsOf:url),original)
        c.session.undo(); XCTAssertTrue(c.session.board.elements.isEmpty); XCTAssertFalse(c.session.canUndo)
    }
    func testDropWithoutCopyPermissionIsRejectedAtEveryDestinationStage() throws {
        let url = try fixture(), c = CanvasController()
        c.pdfPageSelectionPrompt = { _,_ in XCTFail("Rejected drags must never open page selection"); return .page(1) }
        let v = c.canvasView, payload = CanvasDragPayload(urls:[url]); defer { payload.draggingPasteboard.releaseGlobally() }
        for mask: NSDragOperation in [[],.move,.link] {
            payload.draggingSourceOperationMask = mask
            XCTAssertEqual(v.draggingEntered(payload),[])
            XCTAssertFalse(v.prepareForDragOperation(payload)); XCTAssertFalse(v.performDragOperation(payload))
            XCTAssertTrue(c.session.board.elements.isEmpty); XCTAssertFalse(c.session.canUndo)
        }
    }
    func testCancelledAndMixedRemoteDropsDoNotMutateExistingBoard() throws {
        let url = try fixture(), item = CanvasElement(kind:.sticky,frame:CGRect(x:20,y:30,width:200,height:180))
        let c = CanvasController(session:CanvasSession(board:CanvasBoard(elements:[item])))
        c.canvasView.select(item.id); c.pdfPageSelectionPrompt = { _,_ in nil }
        let payload = CanvasDragPayload(urls:[url]); defer { payload.draggingPasteboard.releaseGlobally() }
        XCTAssertFalse(c.canvasView.performDragOperation(payload))
        payload.draggingPasteboard.clearContents()
        payload.draggingPasteboard.writeObjects([url as NSURL,URL(string:"https://example.com/private.pdf")! as NSURL])
        XCTAssertEqual(c.canvasView.draggingUpdated(payload),[])
        XCTAssertFalse(c.canvasView.prepareForDragOperation(payload)); XCTAssertFalse(c.canvasView.performDragOperation(payload))
        XCTAssertEqual(c.session.board.elements,[item]); XCTAssertEqual(c.canvasView.selectedID,item.id)
        XCTAssertFalse(c.session.canUndo); XCTAssertFalse(c.session.isDirty)
    }
}
