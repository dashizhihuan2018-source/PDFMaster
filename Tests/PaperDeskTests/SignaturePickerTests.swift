import XCTest
import AppKit
import PDFKit
import PaperCore
@testable import PaperDesk

@MainActor final class SignaturePickerTests: XCTestCase {
    func testImportReopenSelectAndConfirmedDelete() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let source = root.appendingPathComponent("source.png")
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 8, pixelsHigh: 4, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        let bytes = try XCTUnwrap(bitmap.bitmapData); for i in 0..<(8 * 4 * 4) { bytes[i] = 0 }; bytes[0] = 255; bytes[3] = 255
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: source)
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        var selected: SignatureImage?, confirm = false, errors = [String]()
        let picker = SignaturePickerController(store: store, onSelectImage: { selected = $0 }, onDraw: {}, chooseFile: { source }, confirmDelete: { confirm }, presentError: { errors.append($0) })
        picker.reloadHistory(); picker.importPNG()
        XCTAssertEqual(picker.entries.count, 1)
        let id = try XCTUnwrap(picker.entries.first?.id)
        let reopened = SignaturePickerController(store: store, onSelectImage: { selected = $0 }, onDraw: {}, chooseFile: { nil }, confirmDelete: { confirm }, presentError: { errors.append($0) })
        reopened.reloadHistory(); XCTAssertEqual(reopened.entries.count, 1)
        reopened.select(id: id); let image = try XCTUnwrap(selected)
        let workspace = PDFWorkspace(); try workspace.createBlank()
        try workspace.addImageSignature(image, page: 0, bounds: CGRect(x: 80, y: 100, width: 160, height: 80))
        let pdf = root.appendingPathComponent("signed.pdf"); try workspace.save(to: pdf)
        let beforeDoc = try XCTUnwrap(PDFDocument(url: pdf))
        let before = try XCTUnwrap(beforeDoc.page(at: 0)?.thumbnail(of: CGSize(width: 200, height: 200), for: .cropBox).tiffRepresentation)
        reopened.delete(id: id); XCTAssertEqual(reopened.entries.count, 1)
        confirm = true; reopened.delete(id: id); XCTAssertTrue(reopened.entries.isEmpty)
        XCTAssertTrue(FileManager.default.fileExists(atPath: source.path)); XCTAssertTrue(errors.isEmpty)
        let afterDoc = try XCTUnwrap(PDFDocument(url: pdf))
        let after = try XCTUnwrap(afterDoc.page(at: 0)?.thumbnail(of: CGSize(width: 200, height: 200), for: .cropBox).tiffRepresentation)
        XCTAssertEqual(before, after)
    }

    func testInvalidImportReportsErrorAndKeepsHistory() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let bad = root.appendingPathComponent("bad.png"); try Data("invalid".utf8).write(to: bad)
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        var errors = [String]()
        let picker = SignaturePickerController(store: store, onSelectImage: { _ in XCTFail("invalid image selected") }, onDraw: {}, chooseFile: { bad }, confirmDelete: { false }, presentError: { errors.append($0) })
        picker.reloadHistory(); picker.importPNG()
        XCTAssertEqual(errors.count, 1); XCTAssertTrue(picker.entries.isEmpty)
    }

    func testShellTabsAndNewWindowShareInjectedProfile() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let store = SignatureAssetStore(directory: root.appendingPathComponent("Signatures"))
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: root.appendingPathComponent("recent.json")), signatureStore: store)
        let a = shell.createBlankDocument(), b = shell.createBlankDocument()
        XCTAssertTrue(a.controller.signatureStore === store); XCTAssertTrue(b.controller.signatureStore === store)
        let app = AppDelegate(controller: shell); let session = app.newWindow(makeKey: false)
        XCTAssertTrue(session.controller.signatureStore === store)
        XCTAssertTrue(session.controller.createBlankDocument().controller.signatureStore === store)
        session.window.close()
        let separate = ApplicationShellController(recentStore: RecentDocumentStore(url: root.appendingPathComponent("other/recent.json")))
        XCTAssertFalse(separate.signatureStore === store)
    }
}
