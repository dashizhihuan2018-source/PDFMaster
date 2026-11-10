import XCTest
import AppKit
import PDFKit
import PaperCore
@testable import PaperDesk

@MainActor final class SignaturePickerTests: XCTestCase {
    var root: URL!
    override func setUp() async throws {
        root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    }
    override func tearDown() async throws { try? FileManager.default.removeItem(at: root) }
    func png(_ name: String, value: UInt8 = 120, width: Int = 8, height: Int = 4) throws -> URL {
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        memset(bitmap.bitmapData!, 0, bitmap.bytesPerRow * height)
        bitmap.bitmapData![0] = value; bitmap.bitmapData![3] = 255
        let url = root.appendingPathComponent(name)
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: url)
        return url
    }
    func descendants<T: NSView>(_ view: NSView, _: T.Type) -> [T] {
        ((view as? T).map { [$0] } ?? []) + view.subviews.flatMap { descendants($0, T.self) }
    }
    func testEmbeddedInspectorKeepsSaveOrderAndOnlyPlusAction() throws {
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        let first = try store.importPNG(from: png("first.png", value: 30))
        let second = try store.importPNG(from: png("second.png", value: 80))
        let picker = SignaturePickerController(store: store, onSelectImage: { _ in }, presentError: { XCTFail($0) })
        picker.reloadHistory(); picker.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(picker.entries.map(\.id), [first.id, second.id]); XCTAssertNil(picker.view.window)
        let buttons = descendants(picker.view, NSButton.self)
        XCTAssertEqual(buttons.count, 1)
        let plus = try XCTUnwrap(buttons.first)
        XCTAssertEqual(plus.accessibilityLabel(), "添加签名")
        XCTAssertEqual(plus.frame.size, NSSize(width: 24, height: 24)); XCTAssertEqual(plus.frame.minX, 6, accuracy: 0.1)
        XCTAssertEqual(picker.view.bounds.maxY - plus.frame.maxY, 10, accuracy: 0.1)
        XCTAssertFalse(picker.tableView.allowsMultipleSelection); XCTAssertFalse(picker.tableView.allowsEmptySelection)
        XCTAssertEqual(picker.tableView.rowHeight, 84)
        XCTAssertEqual(picker.tableView.intercellSpacing.height, 8)
        let scroll = try XCTUnwrap(descendants(picker.view, NSScrollView.self).first)
        XCTAssertTrue(scroll.hasVerticalScroller); XCTAssertFalse(scroll.hasHorizontalScroller)
        let separator = try XCTUnwrap(descendants(picker.view, NSBox.self).first)
        XCTAssertEqual(scroll.frame.maxY, separator.frame.minY, accuracy: 0.1)
    }
    func testEmptyMessageAndProportionalAlphaThumbnail() throws {
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        let picker = SignaturePickerController(store: store, onSelectImage: { _ in }, presentError: { XCTFail($0) })
        picker.reloadHistory()
        XCTAssertTrue(descendants(picker.view, NSTextField.self).contains { !$0.isHidden && $0.stringValue.contains("上方") })
        picker.view.layoutSubtreeIfNeeded()
        let empty = try XCTUnwrap(descendants(picker.view, NSTextField.self).first { $0.stringValue.contains("上方") })
        XCTAssertEqual(empty.frame.midY, picker.view.bounds.midY, accuracy: 0.1)
        _ = try store.importPNG(from: png("wide.png", width: 800, height: 200)); picker.reloadHistory()
        let cell = try XCTUnwrap(picker.tableView(picker.tableView, viewFor: nil, row: 0) as? NSTableCellView)
        let imageView = try XCTUnwrap(cell.imageView)
        XCTAssertEqual(imageView.imageScaling, .scaleProportionallyUpOrDown)
        let image = try XCTUnwrap(imageView.image)
        XCTAssertEqual(image.size.width / image.size.height, 4, accuracy: 0.01)
        let cg = try XCTUnwrap(image.cgImage(forProposedRect: nil, context: nil, hints: nil))
        XCTAssertLessThanOrEqual(max(cg.width, cg.height), 240)
        XCTAssertTrue([CGImageAlphaInfo.premultipliedLast, .premultipliedFirst, .first, .last].contains(cg.alphaInfo))
    }
    func testUnavailableRowsStayVisibleAndDeleteImmediatelyWithoutRenderAlerts() throws {
        let directory = root.appendingPathComponent("Signatures")
        let store = SignatureAssetStore(directory: directory)
        let good = try store.importPNG(from: png("good.png", value: 30))
        let source = try png("missing.png", value: 80)
        let missing = try store.importPNG(from: source)
        try FileManager.default.removeItem(at: directory.appendingPathComponent("\(missing.id).png"))
        var selected: SignatureImage?, errors = [String]()
        let picker = SignaturePickerController(store: store, onSelectImage: { selected = $0 }, presentError: { errors.append($0) })
        picker.reloadHistory(); picker.reloadHistory()
        XCTAssertEqual(picker.tableView.numberOfRows, 2); XCTAssertTrue(errors.isEmpty)
        let cell = try XCTUnwrap(picker.tableView(picker.tableView, viewFor: nil, row: 1) as? NSTableCellView)
        XCTAssertTrue(cell.textField?.stringValue.contains("不可用") == true)
        picker.select(id: good.id); XCTAssertNotNil(selected)
        picker.select(id: missing.id); XCTAssertEqual(errors.count, 1)
        picker.delete(id: missing.id)
        XCTAssertEqual(picker.entries.map(\.id), [good.id]); XCTAssertEqual(picker.selectedID, good.id)
        XCTAssertTrue(FileManager.default.fileExists(atPath: source.path))
    }
    func testContextDeleteTargetsClickedRowAndDeleteKeyKeepsSelectionCoherent() throws {
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        let first = try store.importPNG(from: png("first.png", value: 30))
        _ = try store.importPNG(from: png("second.png", value: 80))
        let picker = SignaturePickerController(store: store, onSelectImage: { _ in }, presentError: { XCTFail($0) })
        picker.reloadHistory(); picker.select(id: first.id)
        let menu = try XCTUnwrap(picker.contextMenu(forRow: 1))
        let remove = try XCTUnwrap(menu.items.first)
        XCTAssertEqual(remove.title, "删除")
        XCTAssertTrue(NSApp.sendAction(try XCTUnwrap(remove.action), to: remove.target, from: remove))
        XCTAssertEqual(picker.entries.map(\.id), [first.id])
        let key = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, characters: "\u{7f}", charactersIgnoringModifiers: "\u{7f}", isARepeat: false, keyCode: 51))
        picker.tableView.keyDown(with: key)
        XCTAssertTrue(picker.entries.isEmpty); XCTAssertNil(picker.selectedID)
        XCTAssertNil(picker.contextMenu(forRow: -1))
    }
    func testDeleteDoesNotChangeSourceOrAlreadySignedPDF() throws {
        let source = try png("source.png"), bytes = try Data(contentsOf: source)
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        let entry = try store.importPNG(from: source)
        let workspace = PDFWorkspace(); try workspace.createBlank()
        try workspace.addImageSignature(store.image(for: entry.id), page: 0, bounds: CGRect(x: 80, y: 100, width: 160, height: 80))
        let pdf = root.appendingPathComponent("signed.pdf"); try workspace.save(to: pdf)
        let before = try Data(contentsOf: pdf)
        let picker = SignaturePickerController(store: store, onSelectImage: { _ in }, presentError: { XCTFail($0) })
        picker.reloadHistory(); picker.delete(id: entry.id)
        XCTAssertEqual(try Data(contentsOf: source), bytes); XCTAssertEqual(try Data(contentsOf: pdf), before)
    }
    func testRightInspectorNarrowsPDFAndReusesControllerAcrossActivation() throws {
        let controller = WorkbenchController()
        controller.signatureStore = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        try controller.workspace.createBlank(); _ = controller.view; controller.refreshDocument(); controller.view.layoutSubtreeIfNeeded()
        let before = controller.pdfView.frame
        controller.addSignature(); controller.view.layoutSubtreeIfNeeded()
        let picker = try XCTUnwrap(controller.signaturePicker)
        XCTAssertTrue(controller.isSignatureInspectorVisible); XCTAssertEqual(controller.signatureInspectorFrame.width, 240, accuracy: 0.1)
        XCTAssertEqual(controller.pdfView.frame.maxX, controller.signatureInspectorFrame.minX, accuracy: 0.1)
        XCTAssertEqual(controller.pdfView.frame.width, before.width - 240, accuracy: 0.1)
        XCTAssertEqual(controller.thumbnailSidebarFrame.width, 176, accuracy: 0.1)
        controller.addSignature(); XCTAssertTrue(controller.signaturePicker === picker)
        XCTAssertEqual(controller.children.filter { $0 is SignaturePickerController }.count, 1)
        controller.enterMode(.pages); controller.view.layoutSubtreeIfNeeded(); XCTAssertFalse(controller.isSignatureInspectorVisible)
        controller.enterMode(.annotation); controller.addSignature()
        controller.beginPlacement(.freeText(""), accessibilityName: "text"); XCTAssertFalse(controller.isSignatureInspectorVisible)
    }
    func testNoDocumentAndRestrictedDocumentCannotRevealInspector() throws {
        let controller = WorkbenchController()
        controller.signatureStore = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        _ = controller.view; controller.addSignature()
        XCTAssertFalse(controller.isSignatureInspectorVisible); XCTAssertNil(controller.signaturePicker)
        let seed = PDFWorkspace(); try seed.createBlank()
        let url = root.appendingPathComponent("restricted.pdf")
        XCTAssertTrue(seed.document!.write(to: url, withOptions: [.ownerPasswordOption: "fixture-owner", .accessPermissionsOption: NSNumber(value: 0)]))
        try controller.workspace.open(url); controller.refreshDocument(); XCTAssertFalse(controller.workspace.allowsCommenting)
        controller.addSignature(); XCTAssertFalse(controller.isSignatureInspectorVisible)
    }
    func testSignatureMenuLeavesPageOverviewBeforeRevealingInspector() throws {
        let controller = WorkbenchController()
        controller.signatureStore = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        try controller.workspace.createBlank(); _ = controller.view; controller.refreshDocument()
        controller.enterMode(.pages); XCTAssertTrue(controller.isOverviewVisible)
        controller.addSignature()
        XCTAssertFalse(controller.isOverviewVisible); XCTAssertFalse(controller.pdfView.isHidden)
        XCTAssertTrue(controller.isSignatureInspectorVisible)
    }
    func testShellTabsShareLibraryAndReloadWhenReactivated() throws {
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: root.appendingPathComponent("recent.json")), signatureStore: store)
        let a = shell.createBlankDocument(), b = shell.createBlankDocument()
        a.controller.addSignature(); b.controller.addSignature()
        let asset = try store.importPNG(from: png("new.png"))
        a.controller.addSignature(); XCTAssertEqual(a.controller.signaturePicker?.entries.map(\.id), [asset.id])
        b.controller.addSignature(); XCTAssertTrue(a.controller.signatureStore === b.controller.signatureStore)
        XCTAssertEqual(b.controller.signaturePicker?.entries.map(\.id), [asset.id])
    }
    func testSharedStoreMutationDoesNotRetargetStaleTableSelection() throws {
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        let first = try store.importPNG(from: png("first.png", value: 30))
        let second = try store.importPNG(from: png("second.png", value: 80))
        let picker = SignaturePickerController(store: store, onSelectImage: { _ in }, presentError: { _ in })
        let other = SignaturePickerController(store: store, onSelectImage: { _ in }, presentError: { XCTFail($0) })
        other.reloadHistory()
        picker.reloadHistory(); picker.select(id: first.id)
        other.delete(id: first.id)
        XCTAssertEqual(picker.selectedID, first.id, "Visible rows keep their identity until reload")
        picker.delete(id: first.id)
        XCTAssertEqual(store.entries.map(\.id), [second.id], "A stale deleted row cannot delete its replacement")
        picker.reloadHistory()
        XCTAssertEqual(picker.entries.map(\.id), [second.id]); XCTAssertEqual(picker.selectedID, second.id)
    }
    func testPlusOwnsResettableSheetAndCreateSelectsWithoutClosingInspector() throws {
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        var selected: SignatureImage?
        let picker = SignaturePickerController(store: store, onSelectImage: { selected = $0 }, presentError: { XCTFail($0) })
        let owner = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 240, height: 480), styleMask: [.titled], backing: .buffered, defer: false)
        owner.contentViewController = picker; picker.reloadHistory()
        defer { picker.dismissEditor(); owner.contentViewController = nil }
        let plus = try XCTUnwrap(descendants(picker.view, NSButton.self).first)
        plus.performClick(nil)
        let editor = try XCTUnwrap(picker.editor), sheet = try XCTUnwrap(editor.editorWindow)
        XCTAssertTrue(sheet.sheetParent === owner); XCTAssertEqual(sheet.contentView?.bounds.size, NSSize(width: 480, height: 408))
        XCTAssertTrue(editor.acceptImage(from: try png("new.png"))); XCTAssertTrue(store.entries.isEmpty)
        XCTAssertTrue(editor.create()); XCTAssertNotNil(selected)
        XCTAssertTrue(picker.view.window === owner); XCTAssertEqual(picker.selectedID, store.entries.first?.id)
        XCTAssertNil(editor.editorWindow)
        plus.performClick(nil)
        XCTAssertNil(picker.editor?.draft)
        picker.dismissEditor(); XCTAssertNil(owner.attachedSheet)
    }
}
