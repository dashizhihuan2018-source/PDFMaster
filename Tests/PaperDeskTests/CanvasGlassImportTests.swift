import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor final class CanvasGlassImportTests: XCTestCase {
    private func pdf(pages: Int = 3) throws -> Data {
        let doc = PDFDocument()
        for index in 0..<pages {
            let page = PDFPage()
            page.setBounds(CGRect(x: 0, y: 0, width: 300 + index * 20, height: 400), for: .mediaBox)
            let note = PDFAnnotation(bounds: CGRect(x: 30, y: 30, width: 20, height: 20), forType: .text, withProperties: nil)
            note.contents = "Page \(index + 1)"; page.addAnnotation(note)
            doc.insert(page, at: index)
        }
        return try XCTUnwrap(doc.dataRepresentation())
    }
    func testCanvasUsesFloatingGlassWithoutReducingWorkSurface() {
        let c = CanvasController(); c.view.frame = CGRect(x: 0, y: 0, width: 900, height: 650)
        c.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(c.canvasView.frame, c.view.bounds, "Canvas should fill the view; controls float above it")
        let glass = c.view.subviews.compactMap { $0 as? NSVisualEffectView }
        XCTAssertGreaterThanOrEqual(glass.count, 4)
        XCTAssertTrue(glass.allSatisfy { $0.frame.width < 800 }, "No full-width chrome rows")
    }
    func testDefaultPDFImportEmbedsOnlySelectedPageNotEntireDocument() throws {
        let c = CanvasController(); _ = c.view
        try c.importAsset(data: pdf(), name: "pages.pdf", kind: .pdf)
        let embedded = try XCTUnwrap(PDFDocument(data: try XCTUnwrap(c.session.board.elements.first?.assetData)))
        XCTAssertEqual(embedded.pageCount, 1)
        XCTAssertTrue(try XCTUnwrap(embedded.page(at: 0)).annotations.contains { $0.contents == "Page 1" })
    }
    private func directory() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("CanvasGlassImport-\(UUID())")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }; return url
    }
    private func image() throws -> Data {
        let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 80, pixelsHigh: 40, bitsPerSample: 8,
            samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 320, bitsPerPixel: 32))
        return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
    }
    func testExplicitPageTwoUsesOneBasedSelectionAndPersistsV1() throws {
        let source = try pdf(), c = CanvasController()
        try c.importAsset(data: source, name: "original.pdf", kind: .pdf, selection: .page(2), at: CGPoint(x: 410, y: -30))
        let item = try XCTUnwrap(c.session.board.elements.first)
        XCTAssertEqual(item.frame.midX, 410); XCTAssertEqual(item.frame.midY, -30)
        XCTAssertTrue(try XCTUnwrap(item.assetName).contains("第 2 页"))
        let doc = try XCTUnwrap(PDFDocument(data: try XCTUnwrap(item.assetData)))
        XCTAssertEqual(doc.pageCount, 1)
        XCTAssertTrue(try XCTUnwrap(doc.page(at: 0)).annotations.contains { $0.contents == "Page 2" })
        XCTAssertEqual(PDFDocument(data: source)?.pageCount, 3)
        let restored = try CanvasBoard.decode(c.session.board.encoded())
        XCTAssertEqual(try CanvasController.validatedAssets(restored), c.session.board)
    }
    func testAllPagesHaveIndependentPayloadsNonoverlappingLayoutAndAtomicUndo() throws {
        let c = CanvasController(), bytes = try pdf(pages: 7), frozen = bytes
        try c.importAsset(data: bytes, name: "seven.pdf", kind: .pdf, selection: .allPages, at: CGPoint(x: 100, y: 140))
        XCTAssertEqual(c.session.board.elements.count, 7)
        XCTAssertEqual(c.session.board.elements.first?.frame.midX, 100)
        XCTAssertEqual(c.session.board.elements.first?.frame.midY, 140)
        for (index, item) in c.session.board.elements.enumerated() {
            let doc = try XCTUnwrap(PDFDocument(data: try XCTUnwrap(item.assetData)))
            XCTAssertEqual(doc.pageCount, 1)
            XCTAssertTrue(try XCTUnwrap(doc.page(at: 0)).annotations.contains { $0.contents == "Page \(index+1)" })
            for other in c.session.board.elements where other.id != item.id { XCTAssertFalse(item.frame.intersects(other.frame)) }
        }
        XCTAssertEqual(bytes, frozen)
        c.session.undo(); XCTAssertTrue(c.session.board.elements.isEmpty); XCTAssertFalse(c.session.canUndo)
        c.session.redo(); XCTAssertEqual(c.session.board.elements.count, 7)
    }
    func testFinderImportMixesImagesAndChosenPDFPagesPreservingOriginalFiles() throws {
        let dir = try directory(), pdfURL = dir.appendingPathComponent("local.pdf"), imageURL = dir.appendingPathComponent("local.png")
        let source = try pdf(); try source.write(to: pdfURL); try image().write(to: imageURL)
        let c = CanvasController(); c.pdfPageSelectionPrompt = { name, count in
            XCTAssertEqual(name, "local.pdf"); XCTAssertEqual(count, 3); return .allPages
        }
        XCTAssertTrue(try c.importLocalFiles([imageURL, pdfURL], at: CGPoint(x: -250, y: 380)))
        XCTAssertEqual(c.session.board.elements.map(\.kind), [.image, .pdf, .pdf, .pdf])
        XCTAssertEqual(c.session.board.elements.first?.frame.midX, -250)
        XCTAssertEqual(c.session.board.elements.first?.frame.midY, 380)
        XCTAssertEqual(try Data(contentsOf: pdfURL), source)
        c.session.undo(); XCTAssertTrue(c.session.board.elements.isEmpty); XCTAssertFalse(c.session.canUndo)
    }
    func testCancellationAndLaterCorruptFileLeaveExistingBoardSelectionAndHistoryUntouched() throws {
        let dir = try directory(), good = dir.appendingPathComponent("good.pdf"), bad = dir.appendingPathComponent("bad.png")
        try pdf().write(to: good); try Data("bad".utf8).write(to: bad)
        let item = CanvasElement(kind: .sticky, frame: CGRect(x: 0, y: 0, width: 100, height: 90))
        let c = CanvasController(session: CanvasSession(board: CanvasBoard(elements: [item])))
        c.canvasView.select(item.id); try c.session.mutate { $0.title = "existing undo" }; c.session.undo()
        let before = c.session.board; c.pdfPageSelectionPrompt = { _, _ in nil }
        XCTAssertFalse(try c.importLocalFiles([good])); XCTAssertEqual(c.session.board, before)
        c.pdfPageSelectionPrompt = { _, _ in .allPages }
        XCTAssertThrowsError(try c.importLocalFiles([good, bad]))
        XCTAssertEqual(c.session.board, before); XCTAssertEqual(c.canvasView.selectedID, item.id)
        XCTAssertFalse(c.session.canUndo); XCTAssertTrue(c.session.canRedo); XCTAssertFalse(c.session.isDirty)
    }
    func testInvalidPageEncryptionAndSourcePageCapFailBeforeMutation() throws {
        let c = CanvasController(), source = try pdf()
        for page in [0, -1, 4] { XCTAssertThrowsError(try c.importAsset(data: source, name: "p.pdf", kind: .pdf, selection: .page(page))) }
        let encrypted = try XCTUnwrap(PDFDocument(data: source)?.dataRepresentation(options: [PDFDocumentWriteOption.ownerPasswordOption: "owner", PDFDocumentWriteOption.userPasswordOption: "reader"]))
        XCTAssertThrowsError(try c.importAsset(data: encrypted, name: "locked.pdf", kind: .pdf, selection: .allPages))
        XCTAssertThrowsError(try c.importAsset(data: pdf(pages: 501), name: "too-many.pdf", kind: .pdf, selection: .allPages))
        XCTAssertTrue(c.session.board.elements.isEmpty); XCTAssertFalse(c.session.canUndo)
    }
    func testRejectsOverFileBudgetAndRemoteOrDirectoryInputs() throws {
        let c = CanvasController()
        XCTAssertThrowsError(try c.importAsset(data: Data(repeating: 0, count: CanvasBoard.maxAssetBytes+1), name: "huge.pdf", kind: .pdf))
        XCTAssertThrowsError(try c.importLocalFiles([URL(string: "https://example.com/a.pdf")!]))
        let directoryURL = try directory().appendingPathComponent("folder.pdf")
        try FileManager.default.createDirectory(at: directoryURL, withIntermediateDirectories: true)
        XCTAssertThrowsError(try c.importLocalFiles([directoryURL]))
        XCTAssertFalse(c.session.isDirty); XCTAssertFalse(c.session.canUndo)
    }
    func testTotalBoardBudgetRejectsWholeBatchAndPreservesRedo() throws {
        var largeImage = try image(); largeImage.append(Data(repeating: 0, count: CanvasBoard.maxAssetBytes-largeImage.count))
        let existing = (0..<5).map { index in
            CanvasElement(kind: .image, frame: CGRect(x: index*100, y: 0, width: 80, height: 40), assetData: largeImage, assetName: "existing.png")
        }
        let c = CanvasController(session: CanvasSession(board: CanvasBoard(elements: existing)))
        try c.session.mutate { $0.title = "redo should survive" }; c.session.undo()
        var incoming = try image(); incoming.append(Data(repeating: 0, count: 7*1024*1024-incoming.count))
        let before = c.session.board
        XCTAssertThrowsError(try c.importAsset(data: incoming, name: "too-large-total.png", kind: .image)) {
            XCTAssertEqual($0 as? CanvasBoardError, .resourceLimit)
        }
        XCTAssertEqual(c.session.board, before); XCTAssertFalse(c.session.canUndo); XCTAssertTrue(c.session.canRedo); XCTAssertFalse(c.session.isDirty)
    }
    func testLocalPasteboardAcceptsFilesAndRejectsRemoteMixedTypes() throws {
        let board = NSPasteboard(name: .init("CanvasImportTests-\(UUID())")); defer { board.releaseGlobally() }
        board.writeObjects([URL(fileURLWithPath: "/tmp/local.pdf") as NSURL, URL(fileURLWithPath: "/tmp/local.png") as NSURL])
        XCTAssertEqual(CanvasView.localDropURLs(board)?.count, 2)
        board.clearContents(); board.writeObjects([URL(fileURLWithPath: "/tmp/local.pdf") as NSURL, URL(fileURLWithPath: "/tmp/script.sh") as NSURL])
        XCTAssertNil(CanvasView.localDropURLs(board))
        board.clearContents(); board.writeObjects([URL(string: "https://example.com/p.pdf")! as NSURL])
        XCTAssertNil(CanvasView.localDropURLs(board))
        board.clearContents(); board.writeObjects([URL(fileURLWithPath: "/tmp/local.pdf") as NSURL, URL(string: "https://example.com/p.pdf")! as NSURL])
        XCTAssertNil(CanvasView.localDropURLs(board), "Mixed local/remote input must be rejected, not silently truncated")
        XCTAssertFalse(CanvasView.acceptsLocalAssetURL(URL(string: "file://external-host/a.pdf")!))
    }
    func testIncompleteImageWithValidHeaderCannotEnterOrReplaceBoard() throws {
        let bytes = try image(), truncated = bytes.prefix(40)
        let c = CanvasController()
        XCTAssertThrowsError(try c.importAsset(data: truncated, name: "truncated.png", kind: .image))
        XCTAssertTrue(c.session.board.elements.isEmpty); XCTAssertFalse(c.session.canUndo)
        let malformed = CanvasBoard(elements: [CanvasElement(kind: .image, frame: CGRect(x: 0, y: 0, width: 80, height: 40), assetData: truncated, assetName: "bad.png")])
        XCTAssertThrowsError(try CanvasController.validatedAssets(malformed))
    }
    func testCapsulesFitCompactWindowAndNativeToolButtonsHaveNamesAndStates() {
        let c = CanvasController(); c.view.frame = CGRect(x: 0, y: 0, width: 640, height: 480); c.view.layoutSubtreeIfNeeded()
        let glass = c.view.subviews.compactMap { $0 as? CanvasGlassCapsule }
        for capsule in glass { XCTAssertTrue(c.view.bounds.contains(capsule.frame)); XCTAssertFalse(capsule.mouseDownCanMoveWindow) }
        let tools = glass.first { $0.identifier?.rawValue == "canvas.tools" }!
        func buttons(_ v:NSView) -> [CanvasSymbolButton] { v.subviews.flatMap { ($0 as? CanvasSymbolButton).map { [$0] } ?? buttons($0) } }
        let symbols = buttons(tools)
        XCTAssertEqual(symbols.count, 10)
        XCTAssertTrue(symbols.allSatisfy { !($0.toolTip ?? "").isEmpty && !($0.accessibilityLabel() ?? "").isEmpty })
        XCTAssertEqual(symbols[0].state, .on); symbols[6].performClick(nil)
        XCTAssertEqual(c.canvasView.tool, .connector); XCTAssertEqual(symbols[6].state, .on); XCTAssertEqual(symbols[0].state, .off)
        XCTAssertFalse(c.canvasView.mouseDownCanMoveWindow)
        let undo = buttons(c.view).first { $0.accessibilityLabel() == "撤销" }
        XCTAssertEqual(undo?.isEnabled, false)
        glass.forEach { $0.updateSurface() }
    }
}
