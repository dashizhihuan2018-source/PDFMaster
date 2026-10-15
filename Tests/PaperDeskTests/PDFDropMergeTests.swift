import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor
final class PDFDropMergeTests: XCTestCase {
    func testBusyPageDropReportsFailureWithoutClaimingAccepted() throws {
        let controller = WorkbenchController(); _ = controller.view
        try controller.workspace.createBlank(); controller.refreshDocument(); controller.togglePageOverview()
        controller.localToolCancellation = LocalToolCancellation()
        let drag = PDFFileDraggingInfo(urls: [URL(fileURLWithPath: "/tmp/busy.pdf")])
        XCTAssertFalse(controller.pageOverview.collectionView(controller.pageOverview.collectionView, acceptDrop: drag, indexPath: IndexPath(item: 0, section: 0), dropOperation: .before))
    }

    func testImportHasReachableCancelControlAndSidebarToolWiring() throws {
        let controller = WorkbenchController(); _ = controller.view
        try controller.workspace.createBlank(); controller.refreshDocument()
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        XCTAssertNotNil(descendants(controller.view).first { $0.identifier?.rawValue == "document-file-import-cancel" })
        XCTAssertNotNil(controller.readerSidebar.onAnnotationSelection)
        XCTAssertNotNil(controller.readerSidebar.onAnnotationTool)
    }
    func testHomeMixedTextDropCreatesIndependentTabAndKeepsExistingDocument() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("HomeFiles-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: dir) }
        let txt = dir.appendingPathComponent("home.txt"); try Data("Home mixed input".utf8).write(to: txt)
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: dir.appendingPathComponent("recents.json")))
        _ = shell.view
        let existing = shell.createBlankDocument()
        let imported = try await shell.importDroppedFiles([txt, txt])
        XCTAssertEqual(shell.documentTabs.count, 2)
        XCTAssertEqual(existing.controller.workspace.document?.pageCount, 1)
        XCTAssertEqual(imported.controller.workspace.document?.pageCount, 2)
        XCTAssertNil(imported.sourceURL)
        XCTAssertTrue(imported.controller.workspace.isDirty)
    }

    func testImportCancelledOrChangedDocumentNeverPublishes() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("StaleFiles-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: dir) }
        let txt = dir.appendingPathComponent("large.txt"); try Data(String(repeating: "Read me\n", count: 2000).utf8).write(to: txt)
        for cancel in [true, false] {
            let controller = WorkbenchController(); _ = controller.view
            try controller.workspace.createBlank(); controller.refreshDocument()
            let task = Task { try await controller.mergeDroppedFiles([txt], at: 0) }
            while !controller.isLocalToolRunning { await Task.yield() }
            if cancel { controller.cancelLocalTool() }
            else { try controller.workspace.insertBlank(at: 1) }
            do { try await task.value; XCTFail("Cancelled/stale import was published") } catch {}
            XCTAssertEqual(controller.workspace.document?.pageCount, cancel ? 1 : 2)
            XCTAssertFalse(controller.isLocalToolRunning)
        }
    }

    func testReadingViewReceivesLocalMixedFilesButRejectsRemoteDrop() {
        let view = InkPDFView(frame: NSRect(x: 0, y: 0, width: 600, height: 600))
        let urls = [URL(fileURLWithPath: "/tmp/incoming.png"), URL(fileURLWithPath: "/tmp/incoming.txt")]
        var received: [URL]?
        view.onDropFiles = { received = $0; return true }
        let local = PDFFileDraggingInfo(urls: urls)
        XCTAssertEqual(view.draggingEntered(local), .copy)
        XCTAssertTrue(view.performDragOperation(local))
        XCTAssertEqual(received, urls)
        XCTAssertFalse(view.performDragOperation(PDFFileDraggingInfo(urls: [URL(string: "https://example.test/image.png")!])))
    }

    func testBackgroundTextMergeIsOneUndoAndSavedTextIsSearchable() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("MixedWorkbench-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let text = directory.appendingPathComponent("notes.txt")
        try Data("拖入中文资料".utf8).write(to: text)
        let controller = WorkbenchController(); _ = controller.view
        try controller.workspace.createBlank(); controller.refreshDocument(); controller.togglePageOverview()
        try await controller.mergeDroppedFiles([text, text], at: 0)
        XCTAssertEqual(controller.workspace.document?.pageCount, 3)
        XCTAssertEqual(controller.pageOverview.selectedPages, [0, 1])
        XCTAssertTrue(controller.workspace.document?.page(at: 0)?.string?.contains("拖入中文资料") == true)
        let output = directory.appendingPathComponent("merged.pdf")
        try controller.workspace.saveCopy(to: output)
        XCTAssertEqual(PDFDocument(url: output)?.pageCount, 3)
        XCTAssertTrue(PDFDocument(url: output)?.page(at: 1)?.string?.contains("拖入中文资料") == true)
        controller.undo()
        XCTAssertEqual(controller.workspace.document?.pageCount, 1)
        XCTAssertFalse(controller.isLocalToolRunning)
    }

    func testNativeDropReachesWorkbenchMergeUpdatesGridAndSavesIndependentCopy() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("WorkbenchDrop-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        func fixture(_ name: String) throws -> URL {
            let document = PDFDocument()
            let page = PDFPage()
            page.setBounds(CGRect(x: 0, y: 0, width: 420, height: 594), for: .mediaBox)
            let annotation = PDFAnnotation(bounds: CGRect(x: 30, y: 30, width: 100, height: 30), forType: .freeText, withProperties: nil)
            annotation.contents = name
            page.addAnnotation(annotation)
            document.insert(page, at: 0)
            let url = directory.appendingPathComponent(name + ".pdf")
            XCTAssertTrue(document.write(to: url))
            return url
        }
        let base = try fixture("base"), incoming = try fixture("incoming")
        let originalBytes = try [base, incoming].map { try Data(contentsOf: $0) }
        let controller = WorkbenchController(); _ = controller.view
        try controller.workspace.open(base)
        controller.refreshDocument()
        controller.togglePageOverview()
        let overview = controller.pageOverview
        let drag = PDFFileDraggingInfo(urls: [incoming])
        XCTAssertTrue(overview.collectionView(overview.collectionView, acceptDrop: drag, indexPath: IndexPath(item: 0, section: 0), dropOperation: .before))
        while controller.isLocalToolRunning { await Task.yield() }
        XCTAssertEqual(controller.workspace.document?.pageCount, 2)
        XCTAssertEqual(overview.collectionView(overview.collectionView, numberOfItemsInSection: 0), 2)
        XCTAssertEqual(overview.selectedPages, [0])
        XCTAssertEqual(controller.workspace.document?.page(at: 0)?.annotations.first?.contents, "incoming")
        let output = directory.appendingPathComponent("merged-copy.pdf")
        try controller.workspace.saveCopy(to: output)
        XCTAssertEqual(PDFDocument(url: output)?.pageCount, 2)
        XCTAssertEqual(try [base, incoming].map { try Data(contentsOf: $0) }, originalBytes)
        controller.undo()
        XCTAssertEqual(controller.pdfView.document?.pageCount, 1)
        XCTAssertEqual(overview.collectionView(overview.collectionView, numberOfItemsInSection: 0), 1)
    }

    func testPageOverviewRegistersFinderFileDropsAndNativeCollectionDelegate() {
        let controller = PageOverviewController()
        controller.loadView()
        XCTAssertTrue(controller.collectionView.registeredDraggedTypes.contains(.fileURL))
        XCTAssertTrue(controller.responds(to: NSSelectorFromString("collectionView:validateDrop:proposedIndexPath:dropOperation:")))
        XCTAssertTrue(controller.responds(to: NSSelectorFromString("collectionView:acceptDrop:indexPath:dropOperation:")))
    }

    func testNativeAcceptForwardsEveryPDFInPasteboardOrderWithoutMutatingPresentation() throws {
        let controller = overview()
        let first = URL(fileURLWithPath: "/tmp/Finder First.pdf")
        let second = URL(fileURLWithPath: "/tmp/Finder Second.PDF")
        let drag = PDFFileDraggingInfo(urls: [first, second])
        var received: ([URL], Int)?
        controller.onDropPDFs = { received = ($0, $1); return true }
        XCTAssertTrue(controller.collectionView(controller.collectionView, acceptDrop: drag, indexPath: IndexPath(item: 1, section: 0), dropOperation: .before))
        XCTAssertEqual(received?.0, [first, second])
        XCTAssertEqual(received?.1, 1)
        XCTAssertEqual(controller.collectionView(controller.collectionView, numberOfItemsInSection: 0), 3)
        XCTAssertEqual(controller.selectedPages, [0])
    }

    func testMixedImagesAndTextAreForwardedInPasteboardOrder() {
        let controller = overview()
        let urls = [URL(fileURLWithPath: "/tmp/one.pdf"), URL(fileURLWithPath: "/tmp/two.png"), URL(fileURLWithPath: "/tmp/三.txt")]
        var received: [URL]?
        controller.onDropPDFs = { received = $0; XCTAssertEqual($1, 1); return true }
        XCTAssertTrue(controller.collectionView(controller.collectionView, acceptDrop: PDFFileDraggingInfo(urls: urls), indexPath: IndexPath(item: 1, section: 0), dropOperation: .before))
        XCTAssertEqual(received, urls)
    }

    func testRemoteAndInvalidPasteboardItemsRejectWholeDrop() {
        let controller = overview()
        var callbacks = 0
        controller.onDropPDFs = { _, _ in callbacks += 1; return true }
        for urls in [
            [URL(string: "https://example.test/one.pdf")!],
            [URL(fileURLWithPath: "/tmp/one.pdf"), URL(string: "https://example.test/two.png")!]
        ] {
            let drag = PDFFileDraggingInfo(urls: urls)
            XCTAssertFalse(controller.collectionView(controller.collectionView, acceptDrop: drag, indexPath: IndexPath(item: 0, section: 0), dropOperation: .before))
        }
        XCTAssertEqual(callbacks, 0)
    }

    func testNativeValidationUsesLeftAndRightHalfAsBeforeAndAfterAndBlankAsAppend() throws {
        let controller = overview()
        controller.onDropPDFs = { _, _ in true }
        controller.collectionView.layoutSubtreeIfNeeded()
        let frame = try XCTUnwrap(controller.collectionView.collectionViewLayout?.layoutAttributesForItem(at: IndexPath(item: 1, section: 0))).frame
        for (point, expected) in [
            (CGPoint(x: frame.minX + 8, y: frame.midY), 1),
            (CGPoint(x: frame.maxX - 8, y: frame.midY), 2),
            (CGPoint(x: 5, y: 5), 3)
        ] {
            let drag = PDFFileDraggingInfo(urls: [URL(fileURLWithPath: "/tmp/one.pdf")])
            drag.draggingLocation = controller.collectionView.convert(point, to: nil)
            var proposed: NSIndexPath = IndexPath(item: 1, section: 0) as NSIndexPath
            var operation: NSCollectionView.DropOperation = .on
            let result = controller.collectionView(controller.collectionView, validateDrop: drag, proposedIndexPath: &proposed, dropOperation: &operation)
            XCTAssertEqual(result, .copy)
            XCTAssertEqual(operation, .before)
            XCTAssertEqual(proposed.item, expected)
        }
    }

    func testNoReceiverNoDocumentInternalDragAndNoncopySourceAreRejected() {
        let controller = overview()
        let drag = PDFFileDraggingInfo(urls: [URL(fileURLWithPath: "/tmp/one.pdf")])
        XCTAssertFalse(controller.collectionView(controller.collectionView, acceptDrop: drag, indexPath: IndexPath(item: 0, section: 0), dropOperation: .before))
        controller.onDropPDFs = { _, _ in XCTFail("Invalid drag must not be forwarded"); return false }
        drag.draggingSource = controller.collectionView
        XCTAssertFalse(controller.collectionView(controller.collectionView, acceptDrop: drag, indexPath: IndexPath(item: 0, section: 0), dropOperation: .before))
        drag.draggingSource = nil
        drag.draggingSourceOperationMask = .move
        XCTAssertFalse(controller.collectionView(controller.collectionView, acceptDrop: drag, indexPath: IndexPath(item: 0, section: 0), dropOperation: .before))
        drag.draggingSourceOperationMask = .copy
        XCTAssertFalse(controller.collectionView(controller.collectionView, acceptDrop: drag, indexPath: IndexPath(item: 99, section: 0), dropOperation: .before))
        controller.setDocument(nil, selectedPage: 0)
        XCTAssertFalse(controller.collectionView(controller.collectionView, acceptDrop: drag, indexPath: IndexPath(item: 0, section: 0), dropOperation: .before))
    }

    private func overview() -> PageOverviewController {
        let controller = PageOverviewController()
        controller.loadView()
        let document = PDFDocument()
        for index in 0..<3 { document.insert(PDFPage(), at: index) }
        controller.setDocument(document, selectedPage: 0)
        controller.view.layoutSubtreeIfNeeded()
        return controller
    }
}

/// Only the native session transport is doubled; pasteboard and collection layout are real.
@MainActor
private final class PDFFileDraggingInfo: NSObject, NSDraggingInfo {
    var draggingDestinationWindow: NSWindow? { nil }
    var draggingSourceOperationMask: NSDragOperation = .copy
    var draggingLocation = NSPoint.zero
    var draggedImageLocation: NSPoint { .zero }
    nonisolated var draggedImage: NSImage? { nil }
    let draggingPasteboard: NSPasteboard
    var draggingSource: Any?
    var draggingSequenceNumber: Int { 1 }
    var draggingFormation: NSDraggingFormation = .none
    var animatesToDestination = false
    var numberOfValidItemsForDrop = 0
    var springLoadingHighlight: NSSpringLoadingHighlight { .none }

    init(urls: [URL]) {
        draggingPasteboard = NSPasteboard(name: NSPasteboard.Name("PDFDropMergeTests-\(UUID().uuidString)"))
        super.init()
        let items = urls.map { url -> NSPasteboardItem in
            let item = NSPasteboardItem()
            item.setString(url.absoluteString, forType: .fileURL)
            return item
        }
        draggingPasteboard.writeObjects(items)
    }
    deinit { draggingPasteboard.releaseGlobally() }
    func slideDraggedImage(to screenPoint: NSPoint) {}
    nonisolated override func namesOfPromisedFilesDropped(atDestination dropDestination: URL) -> [String]? { nil }
    func resetSpringLoading() {}
    func enumerateDraggingItems(options enumOpts: NSDraggingItemEnumerationOptions, for view: NSView?, classes classArray: [AnyClass], searchOptions: [NSPasteboard.ReadingOptionKey: Any], using block: (NSDraggingItem, Int, UnsafeMutablePointer<ObjCBool>) -> Void) {}
}
