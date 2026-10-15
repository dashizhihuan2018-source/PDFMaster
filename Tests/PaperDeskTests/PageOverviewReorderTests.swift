import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor
final class PageOverviewReorderTests: XCTestCase {
    func testOverviewProvidesNativePageDragWriterAndRegistersLocalMovePayload() {
        let controller = PageOverviewController()
        controller.loadView()
        XCTAssertTrue(controller.responds(to: NSSelectorFromString("collectionView:pasteboardWriterForItemAtIndexPath:")))
        XCTAssertTrue(controller.collectionView.registeredDraggedTypes.contains(NSPasteboard.PasteboardType("cn.nankong.pdfmaster.page-overview-pages")))
    }

    func testOverviewExposesNativeDraggingSessionLifecycleCallbacksForDiagnostics() {
        let controller = PageOverviewController()
        XCTAssertTrue(controller.responds(to: NSSelectorFromString("collectionView:draggingSession:willBeginAtPoint:forItemsAtIndexPaths:")))
        XCTAssertTrue(controller.responds(to: NSSelectorFromString("collectionView:draggingSession:endedAtPoint:dragOperation:")))
    }

    func testUserDefaultsEnablesContentFreeAppendOnlyDragDiagnostics() throws {
        let key = "PDFMasterOverviewDragDiagnostics"
        let defaults = UserDefaults.standard
        let previous = defaults.object(forKey: key)
        defaults.set(true, forKey: key)
        defer {
            if let previous { defaults.set(previous, forKey: key) }
            else { defaults.removeObject(forKey: key) }
        }
        let url = URL(fileURLWithPath: "/tmp/pdfmaster-overview-drag-diagnostics.log")
        let before = (try? Data(contentsOf: url)) ?? Data()
        let controller = overview()
        controller.onMovePages = { _, _ in true }
        _ = try writer(controller, page: 0)
        let after = (try? Data(contentsOf: url)) ?? Data()
        XCTAssertTrue(after.starts(with: before), "Diagnostics must only append to existing logs")
        let appended = String(decoding: after.dropFirst(before.count), as: UTF8.self)
        XCTAssertTrue(appended.contains("[PageOverviewDrag] document-changed"))
        XCTAssertTrue(appended.contains("[PageOverviewDrag] writer ready"))
        XCTAssertFalse(appended.contains("sourceID"))
        XCTAssertFalse(appended.contains("file://"))
        XCTAssertFalse(appended.contains("/Users/"))
    }

    func testNativeHitTargetsOnPageItemCannotStartBackgroundWindowDrag() throws {
        let controller = overview()
        let item = controller.collectionView(controller.collectionView, itemForRepresentedObjectAt: IndexPath(item: 0, section: 0))
        let root = item.view
        root.frame = NSRect(x: 0, y: 0, width: 180, height: 300)
        root.layoutSubtreeIfNeeded()
        let thumbnail = try XCTUnwrap(root.subviews.first { $0 is NSImageView })
        let pageLabel = try XCTUnwrap(root.subviews.first { $0 is NSTextField })
        for (name, point) in [("border", CGPoint(x: 2, y: 150)), ("thumbnail", CGPoint(x: thumbnail.frame.midX, y: thumbnail.frame.midY)), ("page label", CGPoint(x: pageLabel.frame.midX, y: pageLabel.frame.midY))] {
            let hit = try XCTUnwrap(root.hitTest(point))
            print("OVERVIEW_NATIVE_HIT name=\(name) class=\(type(of: hit)) mouseDownCanMoveWindow=\(hit.mouseDownCanMoveWindow)")
            XCTAssertFalse(hit.mouseDownCanMoveWindow, "\(name) must route mouse tracking to the collection, not window dragging")
        }
    }

    func testDoubleClickRecognizerDoesNotDelayPrimaryMouseDragEvents() throws {
        let controller = overview()
        let recognizer = try XCTUnwrap(controller.collectionView.gestureRecognizers.compactMap { $0 as? NSClickGestureRecognizer }.first)
        print("OVERVIEW_DOUBLE_CLICK delayPrimary=\(recognizer.delaysPrimaryMouseButtonEvents) clicks=\(recognizer.numberOfClicksRequired)")
        XCTAssertEqual(recognizer.numberOfClicksRequired, 2)
        XCTAssertFalse(recognizer.delaysPrimaryMouseButtonEvents)
    }

    func testNativeMultiPageDropForwardsOriginalInsertionBoundaryAndNeverMutatesPresentation() throws {
        for insertion in [0, 2, 5] {
            let controller = overview()
            var received: (IndexSet, Int)?
            controller.onMovePages = { received = ($0, $1); return true }
            let drag = try pageDrag(controller, pages: [3, 1])
            XCTAssertTrue(accept(controller, drag: drag, insertion: insertion))
            XCTAssertEqual(received?.0, IndexSet([1, 3]))
            XCTAssertEqual(received?.1, insertion)
            XCTAssertEqual(controller.collectionView(controller.collectionView, numberOfItemsInSection: 0), 5)
            XCTAssertEqual(controller.selectedPages, [0])
        }
    }

    func testNativeMoveValidationUsesBeforeAfterAndBackgroundAppend() throws {
        let controller = overview()
        controller.onMovePages = { _, _ in true }
        let frame = try XCTUnwrap(controller.collectionView.collectionViewLayout?.layoutAttributesForItem(at: IndexPath(item: 1, section: 0))).frame
        for (point, expected) in [
            (CGPoint(x: frame.minX + 8, y: frame.midY), 1),
            (CGPoint(x: frame.maxX - 8, y: frame.midY), 2),
            (CGPoint(x: 5, y: 5), 5)
        ] {
            // Use a noncontiguous group so every boundary changes page order.
            let drag = try pageDrag(controller, pages: [0, 3])
            drag.draggingLocation = controller.collectionView.convert(point, to: nil)
            var proposed: NSIndexPath = IndexPath(item: 1, section: 0) as NSIndexPath
            var operation: NSCollectionView.DropOperation = .on
            XCTAssertEqual(controller.collectionView(controller.collectionView, validateDrop: drag, proposedIndexPath: &proposed, dropOperation: &operation), .move)
            XCTAssertEqual(operation, .before)
            XCTAssertEqual(proposed.item, expected)
        }
    }

    func testValidationPreservesNativeBeforeBoundaryAndRefusesNoOpMove() throws {
        let controller = overview()
        controller.onMovePages = { _, _ in true }
        let frame = try XCTUnwrap(controller.collectionView.collectionViewLayout?.layoutAttributesForItem(at: IndexPath(item: 1, section: 0))).frame
        let drag = try pageDrag(controller, pages: [1, 2])
        drag.draggingLocation = controller.collectionView.convert(CGPoint(x: frame.midX, y: frame.midY), to: nil)
        var proposed: NSIndexPath = IndexPath(item: 1, section: 0) as NSIndexPath
        var operation: NSCollectionView.DropOperation = .before
        XCTAssertEqual(controller.collectionView(controller.collectionView, validateDrop: drag, proposedIndexPath: &proposed, dropOperation: &operation), [])
        proposed = IndexPath(item: 0, section: 0) as NSIndexPath
        XCTAssertEqual(controller.collectionView(controller.collectionView, validateDrop: drag, proposedIndexPath: &proposed, dropOperation: &operation), .move)
        XCTAssertEqual(proposed.item, 0)
        XCTAssertEqual(operation, .before)
    }

    func testContiguousAndAllPageNoOpDropsDoNotReachWorkspace() throws {
        let controller = overview()
        controller.onMovePages = { _, _ in XCTFail("No-op must not create an undo entry"); return true }
        for (pages, boundaries) in [([1, 2], [1, 2, 3]), ([0, 1, 2, 3, 4], Array(0...5))] {
            let drag = try pageDrag(controller, pages: pages)
            for insertion in boundaries { XCTAssertFalse(accept(controller, drag: drag, insertion: insertion)) }
        }
    }

    func testStaleForeignCopyOnlyMalformedAndDuplicatePagePayloadsAreRejected() throws {
        let controller = overview()
        controller.onMovePages = { _, _ in XCTFail("Invalid move must not reach workspace"); return true }
        let stale = try pageDrag(controller, pages: [1])
        controller.setDocument(document(), selectedPage: 0)
        XCTAssertFalse(accept(controller, drag: stale, insertion: 5))

        let foreign = try pageDrag(controller, pages: [1])
        foreign.draggingSource = NSCollectionView()
        XCTAssertFalse(accept(controller, drag: foreign, insertion: 5))
        let copy = try pageDrag(controller, pages: [1])
        copy.draggingSourceOperationMask = .copy
        XCTAssertFalse(accept(controller, drag: copy, insertion: 5))

        let other = overview()
        other.onMovePages = { _, _ in true }
        let wrongOwner = try pageDrag(other, pages: [1])
        wrongOwner.draggingSource = controller.collectionView
        XCTAssertFalse(accept(controller, drag: wrongOwner, insertion: 5))

        let duplicate = try pageDrag(controller, pages: [1, 1])
        XCTAssertFalse(accept(controller, drag: duplicate, insertion: 5))
        let malformed = try pageDrag(controller, pages: [1])
        let invalid = NSPasteboardItem()
        invalid.setString("invalid", forType: NSPasteboard.PasteboardType("cn.nankong.pdfmaster.page-overview-pages"))
        malformed.draggingPasteboard.clearContents()
        malformed.draggingPasteboard.writeObjects([invalid])
        XCTAssertFalse(accept(controller, drag: malformed, insertion: 5))
    }

    func testReceiverFailureAndInvalidDestinationAreNotClaimedAccepted() throws {
        let controller = overview()
        var calls = 0
        controller.onMovePages = { _, _ in calls += 1; return false }
        let drag = try pageDrag(controller, pages: [1])
        XCTAssertFalse(accept(controller, drag: drag, insertion: 5))
        XCTAssertEqual(calls, 1)
        XCTAssertFalse(accept(controller, drag: drag, insertion: -1))
        XCTAssertFalse(accept(controller, drag: drag, insertion: 6))
        XCTAssertFalse(controller.collectionView(controller.collectionView, acceptDrop: drag, indexPath: IndexPath(item: 0, section: 1), dropOperation: .before))
        XCTAssertFalse(controller.collectionView(controller.collectionView, acceptDrop: drag, indexPath: IndexPath(item: 0, section: 0), dropOperation: .on))
        XCTAssertEqual(calls, 1)
        controller.onMovePages = nil
        XCTAssertNil(controller.collectionView(controller.collectionView, pasteboardWriterForItemAt: IndexPath(item: 0, section: 0)))
        XCTAssertFalse(accept(controller, drag: drag, insertion: 5))
    }

    func testDragWriterRejectsInvalidPageSectionAndMissingDocument() {
        let controller = overview()
        controller.onMovePages = { _, _ in true }
        for path in [IndexPath(item: 5, section: 0), IndexPath(item: 0, section: 1)] {
            XCTAssertNil(controller.collectionView(controller.collectionView, pasteboardWriterForItemAt: path))
        }
        controller.setDocument(nil, selectedPage: 0)
        XCTAssertNil(controller.collectionView(controller.collectionView, pasteboardWriterForItemAt: IndexPath(item: 0, section: 0)))
    }

    private func writer(_ controller: PageOverviewController, page: Int) throws -> NSPasteboardWriting {
        try XCTUnwrap(controller.collectionView(controller.collectionView, pasteboardWriterForItemAt: IndexPath(item: page, section: 0)))
    }

    private func pageDrag(_ controller: PageOverviewController, pages: [Int]) throws -> OverviewPageDraggingInfo {
        let drag = OverviewPageDraggingInfo(source: controller.collectionView)
        XCTAssertTrue(drag.draggingPasteboard.writeObjects(try pages.map { try writer(controller, page: $0) }))
        return drag
    }

    private func accept(_ controller: PageOverviewController, drag: OverviewPageDraggingInfo, insertion: Int) -> Bool {
        controller.collectionView(controller.collectionView, acceptDrop: drag, indexPath: IndexPath(item: insertion, section: 0), dropOperation: .before)
    }

    private func overview() -> PageOverviewController {
        let controller = PageOverviewController()
        controller.loadView()
        controller.setDocument(document(), selectedPage: 0)
        controller.view.layoutSubtreeIfNeeded()
        return controller
    }

    private func document() -> PDFDocument {
        let document = PDFDocument()
        for index in 0..<5 { document.insert(PDFPage(), at: index) }
        return document
    }
}

/// Only native drag-session transport is doubled; AppKit pasteboard and layout are real.
@MainActor
private final class OverviewPageDraggingInfo: NSObject, NSDraggingInfo {
    var draggingDestinationWindow: NSWindow? { nil }
    var draggingSourceOperationMask: NSDragOperation = .move
    var draggingLocation = NSPoint.zero
    var draggedImageLocation: NSPoint { .zero }
    nonisolated var draggedImage: NSImage? { nil }
    let draggingPasteboard = NSPasteboard(name: NSPasteboard.Name("OverviewReorder-\(UUID().uuidString)"))
    var draggingSource: Any?
    var draggingSequenceNumber: Int { 1 }
    var draggingFormation: NSDraggingFormation = .none
    var animatesToDestination = false
    var numberOfValidItemsForDrop = 0
    var springLoadingHighlight: NSSpringLoadingHighlight { .none }

    init(source: NSCollectionView) { draggingSource = source; super.init() }
    deinit { draggingPasteboard.releaseGlobally() }
    func slideDraggedImage(to screenPoint: NSPoint) {}
    nonisolated override func namesOfPromisedFilesDropped(atDestination dropDestination: URL) -> [String]? { nil }
    func resetSpringLoading() {}
    func enumerateDraggingItems(options enumOpts: NSDraggingItemEnumerationOptions, for view: NSView?, classes classArray: [AnyClass], searchOptions: [NSPasteboard.ReadingOptionKey: Any], using block: (NSDraggingItem, Int, UnsafeMutablePointer<ObjCBool>) -> Void) {}
}
