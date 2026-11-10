import XCTest
import AppKit
import Darwin
import PaperCore
@testable import PaperDesk

@MainActor final class SignatureEditorTests: XCTestCase {
    var root: URL!
    override func setUp() async throws {
        root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    }
    override func tearDown() async throws { try? FileManager.default.removeItem(at: root) }
    func png() throws -> URL {
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 80, pixelsHigh: 20, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        memset(bitmap.bitmapData!, 0, bitmap.bytesPerRow * bitmap.pixelsHigh); bitmap.bitmapData![3] = 255
        let url = root.appendingPathComponent("source.png")
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: url)
        return url
    }
    func testChoosingSnapshotDefersCommitAndSurvivesSourceRemoval() throws {
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures")), source = try png()
        var committed: SignatureAsset?
        let editor = SignatureEditorController(store: store, onCreate: { committed = $0 }, chooseFile: { source }, presentError: { XCTFail($0) })
        _ = editor.view; editor.chooseImage()
        let data = try XCTUnwrap(editor.draft?.pngData)
        XCTAssertTrue(store.entries.isEmpty); XCTAssertNil(committed); XCTAssertEqual(editor.preview.image?.size.width, 80)
        try FileManager.default.removeItem(at: source)
        XCTAssertTrue(editor.create())
        let asset = try XCTUnwrap(committed)
        XCTAssertEqual(store.entries.count, 1); XCTAssertEqual(try store.image(for: asset.id).pngData, data)
    }
    func testInvalidDropPreservesValidDraftAndReportsOnce() throws {
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        var errors = [String]()
        let editor = SignatureEditorController(store: store, onCreate: { _ in XCTFail() }, presentError: { errors.append($0) })
        XCTAssertTrue(editor.acceptImage(from: try png())); let before = editor.draft?.pngData
        let invalid = root.appendingPathComponent("bad.png"); try Data("bad".utf8).write(to: invalid)
        XCTAssertFalse(editor.acceptImage(from: invalid)); XCTAssertEqual(editor.draft?.pngData, before)
        XCTAssertEqual(errors.count, 1); XCTAssertTrue(store.entries.isEmpty)
        XCTAssertTrue(editor.dropView.registeredDraggedTypes.contains(.fileURL))
    }
    func testCancelAndCloseDiscardDraftAndReopeningResets() throws {
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        let editor = SignatureEditorController(store: store, onCreate: { _ in XCTFail() }, presentError: { XCTFail($0) })
        XCTAssertTrue(editor.acceptImage(from: try png())); editor.cancel()
        XCTAssertNil(editor.draft); XCTAssertTrue(store.entries.isEmpty)
        XCTAssertTrue(editor.acceptImage(from: try png()))
        let window = NSWindow(contentRect: .zero, styleMask: [.titled], backing: .buffered, defer: false)
        XCTAssertTrue(editor.windowShouldClose(window)); XCTAssertNil(editor.draft)
        editor.canvas.strokes = [[.zero, CGPoint(x: 1, y: 1)]]; editor.resetDraft()
        XCTAssertTrue(editor.canvas.strokes.isEmpty); XCTAssertEqual(editor.view.frame.size, NSSize(width: 480, height: 408))
    }
    func testFailedCommitKeepsDraftEditableAndHistoryUnchanged() throws {
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"), byteLimit: 0)
        var errors = [String]()
        let editor = SignatureEditorController(store: store, onCreate: { _ in XCTFail() }, presentError: { errors.append($0) })
        XCTAssertTrue(editor.acceptImage(from: try png())); let data = editor.draft?.pngData
        XCTAssertFalse(editor.create()); XCTAssertEqual(errors.count, 1); XCTAssertEqual(editor.draft?.pngData, data)
        XCTAssertTrue(store.entries.isEmpty); try store.reload(); XCTAssertTrue(store.entries.isEmpty)
    }
    func testDrawingCreatesTransparentPNGLibraryOnlyAfterCreate() throws {
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        var created: SignatureAsset?
        let editor = SignatureEditorController(store: store, onCreate: { created = $0 }, presentError: { XCTFail($0) })
        editor.setMode(.draw); editor.canvas.strokes = [[CGPoint(x: 0.2, y: 0.2), CGPoint(x: 0.8, y: 0.8)]]
        editor.view.layoutSubtreeIfNeeded()
        let aspect = editor.canvas.bounds.width / editor.canvas.bounds.height
        XCTAssertTrue(store.entries.isEmpty); XCTAssertTrue(editor.create())
        let image = try store.image(for: XCTUnwrap(created).id), rep = try XCTUnwrap(NSBitmapImageRep(data: image.pngData))
        XCTAssertEqual(rep.colorAt(x: 0, y: 0)?.alphaComponent, 0); XCTAssertTrue(rep.hasAlpha)
        XCTAssertEqual(CGFloat(image.pixelWidth) / CGFloat(image.pixelHeight), aspect, accuracy: 0.01)
    }
    func testDraftReadRejectsLinksFIFOAndOversizeWithoutChangingLibrary() throws {
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        let source = try png(), link = root.appendingPathComponent("link.png"), fifo = root.appendingPathComponent("fifo.png")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: source); XCTAssertEqual(mkfifo(fifo.path, 0o600), 0)
        XCTAssertThrowsError(try store.draft(from: link)); XCTAssertThrowsError(try store.draft(from: fifo))
        let oversized = root.appendingPathComponent("large.png"); try Data(repeating: 0, count: 8 * 1024 * 1024 + 1).write(to: oversized)
        XCTAssertThrowsError(try store.draft(from: oversized)); XCTAssertTrue(store.entries.isEmpty)
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.appendingPathComponent("Signatures").path))
    }
    func testDraftCommitUsesExistingDedupAndRollback() throws {
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures")), draft = try store.draft(from: png())
        let first = try store.save(draft), second = try store.save(draft)
        XCTAssertEqual(first.id, second.id); XCTAssertEqual(store.entries.count, 1)
        let failing = SignatureAssetStore(directory: root.appendingPathComponent("Failure"), renameFile: { a, b in b.hasSuffix("index.json") ? -1 : Darwin.rename(a, b) })
        XCTAssertThrowsError(try failing.save(draft)); XCTAssertTrue(failing.entries.isEmpty)
        XCTAssertFalse(try FileManager.default.contentsOfDirectory(atPath: root.appendingPathComponent("Failure").path).contains { $0.hasSuffix(".png") })
    }
    func testMissingHistoryAssetDoesNotBlockCreationOfNewHealthySignature() throws {
        let directory = root.appendingPathComponent("Signatures"), store = SignatureAssetStore(directory: directory)
        let source = try png(), old = try store.importPNG(from: source)
        try FileManager.default.removeItem(at: directory.appendingPathComponent("\(old.id).png"))
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: Data(contentsOf: source)))
        bitmap.bitmapData![0] = 255
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: source)
        let new = try store.save(store.draft(from: source))
        XCTAssertEqual(store.entries.map(\.id), [old.id, new.id])
        XCTAssertNoThrow(try store.image(for: new.id))
    }
    func testFileDragDestinationAcceptsPNGSnapshotButDoesNotCommit() throws {
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        let editor = SignatureEditorController(store: store, onCreate: { _ in XCTFail() }, presentError: { XCTFail($0) })
        _ = editor.view
        let payload = SignatureDragPayload(urls: [try png()])
        defer { payload.draggingPasteboard.releaseGlobally() }
        XCTAssertEqual(editor.dropView.draggingEntered(payload), .copy)
        XCTAssertTrue(editor.dropView.prepareForDragOperation(payload))
        XCTAssertTrue(editor.dropView.performDragOperation(payload))
        XCTAssertNotNil(editor.draft); XCTAssertTrue(store.entries.isEmpty)
        payload.draggingPasteboard.clearContents()
        payload.draggingPasteboard.writeObjects([URL(string: "https://example.com/test.png")! as NSURL])
        XCTAssertEqual(editor.dropView.draggingEntered(payload), [])
        XCTAssertFalse(editor.dropView.performDragOperation(payload))
        XCTAssertNotNil(editor.draft)
    }
}

/// Destination/pasteboard coverage, not an OS Finder drag end-to-end claim.
@MainActor private final class SignatureDragPayload: NSObject, NSDraggingInfo {
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
        draggingPasteboard = NSPasteboard(name: .init("SignatureDrag-\(UUID())"))
        super.init(); draggingPasteboard.writeObjects(urls.map { $0 as NSURL })
    }
    func slideDraggedImage(to screenPoint: NSPoint) {}
    nonisolated override func namesOfPromisedFilesDropped(atDestination dropDestination: URL) -> [String]? { nil }
    func resetSpringLoading() {}
    func enumerateDraggingItems(options: NSDraggingItemEnumerationOptions, for view: NSView?, classes: [AnyClass], searchOptions: [NSPasteboard.ReadingOptionKey: Any], using block: (NSDraggingItem, Int, UnsafeMutablePointer<ObjCBool>) -> Void) {}
}
