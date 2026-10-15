import XCTest
import PDFKit
import AppKit
@testable import PaperCore

final class FilePDFImportTests: XCTestCase {
    @MainActor func testNewMergeUsesSameAdmissionForEveryPositionAndHasNoPlaceholderUndo() throws {
        let doc = PDFDocument(); let page = PDFPage()
        page.setBounds(CGRect(x: 0, y: 0, width: 300, height: 400), for: .mediaBox)
        let link = PDFAnnotation(bounds: CGRect(x: 10, y: 10, width: 80, height: 20), forType: .link, withProperties: nil)
        link.url = URL(string: "https://example.test/reference"); page.addAnnotation(link)
        doc.insert(page, at: 0)
        let linked = try XCTUnwrap(doc.dataRepresentation())
        let plain = PDFWorkspace(); try plain.createBlank()
        let blank = try XCTUnwrap(plain.document?.dataRepresentation())
        for batch in [[linked, blank], [blank, linked]] {
            let workspace = PDFWorkspace(); try workspace.createUnsavedFileMerge(batch)
            XCTAssertEqual(workspace.document?.pageCount, 2)
            XCTAssertNil(workspace.sourceURL); XCTAssertTrue(workspace.isDirty)
            XCTAssertFalse(workspace.canUndo)
            XCTAssertEqual((0..<2).flatMap { workspace.document!.page(at: $0)!.annotations }.filter { $0.type == "Link" }.count, 1)
            let identity = workspace.document
            XCTAssertThrowsError(try workspace.createUnsavedFileMerge([blank, Data("bad".utf8)]))
            XCTAssertTrue(workspace.document === identity)
        }
    }
    func testMixedPDFImageAndTextBecomeOrderedIndependentPDFs() throws {
        let dir = try directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let base = dir.appendingPathComponent("base.pdf")
        let doc = PDFDocument(); let page = PDFPage(); page.setBounds(CGRect(x: 0, y: 0, width: 300, height: 400), for: .mediaBox)
        doc.insert(page, at: 0); XCTAssertTrue(doc.write(to: base))
        let png = try image(dir)
        let txt = dir.appendingPathComponent("notes.txt"); try Data("中文资料 Mixed files".utf8).write(to: txt)
        let originals = try [base, png, txt].map { try Data(contentsOf: $0) }
        let results = try FilePDFImport.prepare([base, png, txt])
        XCTAssertEqual(results.count, 3)
        guard results.count == 3 else { return }
        XCTAssertEqual(results[0], originals[0])
        XCTAssertEqual(results.compactMap { PDFDocument(data: $0)?.pageCount }, [1, 1, 1])
        XCTAssertTrue(PDFDocument(data: results[2])?.string?.contains("中文资料") == true)
        XCTAssertEqual(PDFDocument(data: results[1])?.page(at: 0)?.bounds(for: .mediaBox).width ?? 0, 595.276, accuracy: 1)
        XCTAssertEqual(try [base, png, txt].map { try Data(contentsOf: $0) }, originals)
    }

    func testUnsupportedCorruptSymlinkDirectoryRemoteAndCancelledImportsFail() throws {
        let dir = try directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let png = try image(dir)
        let unsupported = dir.appendingPathComponent("video.mp4"); try Data([1, 2, 3]).write(to: unsupported)
        let corrupt = dir.appendingPathComponent("bad.png"); try Data("not a picture".utf8).write(to: corrupt)
        let link = dir.appendingPathComponent("link.png"); try FileManager.default.createSymbolicLink(at: link, withDestinationURL: png)
        for item in [unsupported, corrupt, link, dir, URL(string: "https://example.test/test.png")!] {
            XCTAssertThrowsError(try FilePDFImport.prepare([png, item]))
        }
        XCTAssertThrowsError(try FilePDFImport.prepare([png], cancelled: { true }))
        XCTAssertThrowsError(try FilePDFImport.prepare([]))
    }

    private func directory() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("FileImportTests-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: false); return url
    }
    private func image(_ dir: URL) throws -> URL {
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 80, pixelsHigh: 40, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        for x in 0..<80 { for y in 0..<40 { bitmap.setColor(NSColor(deviceRed: 0.1, green: 0.3, blue: 0.8, alpha: 1), atX: x, y: y) } }
        let url = dir.appendingPathComponent("blue.png")
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: url); return url
    }
}
