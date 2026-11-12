import CoreGraphics
import PDFKit
import XCTest
@testable import PaperCore

@MainActor final class BlankPageInsertionTests: XCTestCase {
    func testInsertedBlankUsesPreviousPageGeometryAndSurvivesSaveUndoRedo() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("BlankGeometry-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: folder) }
        let source = folder.appendingPathComponent("mixed.pdf")
        let boxes = [CGRect(x: 10, y: 20, width: 600, height: 300), CGRect(x: -15, y: 25, width: 300, height: 700), CGRect(x: 0, y: 0, width: 420, height: 420)]
        let raw = NSMutableData()
        let context = try XCTUnwrap(CGContext(consumer: try XCTUnwrap(CGDataConsumer(data: raw)), mediaBox: nil, nil))
        for var box in boxes {
            context.beginPage(mediaBox: &box)
            context.setFillColor(CGColor(red: 0.8, green: 0.2, blue: 0.3, alpha: 1)); context.fill(box)
            context.endPage()
        }
        context.closePDF()
        let fixture = try XCTUnwrap(PDFDocument(data: raw as Data))
        let portrait = try XCTUnwrap(fixture.page(at: 1))
        portrait.rotation = 90
        portrait.setBounds(CGRect(x: 0, y: 40, width: 260, height: 600), for: .cropBox)
        portrait.setBounds(CGRect(x: 5, y: 45, width: 250, height: 580), for: .trimBox)
        for index in 0..<3 {
            let note = PDFAnnotation(bounds: CGRect(x: 40, y: 60, width: 20, height: 20), forType: .text, withProperties: nil)
            note.contents = "SOURCE-\(index)"; fixture.page(at: index)!.addAnnotation(note)
        }
        XCTAssertTrue(fixture.write(to: source))
        let original = try Data(contentsOf: source)
        for slot in [0, 1, 2, 3] {
            let workspace = PDFWorkspace(); try workspace.open(source)
            let previous = try XCTUnwrap(workspace.document?.page(at: max(0, slot - 1)))
            let originalBoxes = (0..<3).map { workspace.document!.page(at: $0)!.bounds(for: .mediaBox) }
            try workspace.insertBlank(at: slot)
            let blank = try XCTUnwrap(workspace.document?.page(at: slot))
            assertGeometry(blank, matches: previous)
            XCTAssertTrue(blank.annotations.isEmpty)
            XCTAssertTrue(blank.string?.isEmpty ?? true)
            for old in 0..<3 {
                let page = workspace.document!.page(at: old < slot ? old : old + 1)!
                XCTAssertEqual(page.bounds(for: .mediaBox), originalBoxes[old])
                XCTAssertEqual(page.annotations.first?.contents, "SOURCE-\(old)")
            }
            let copy = folder.appendingPathComponent("slot-\(slot).pdf")
            try workspace.saveCopy(to: copy)
            let reopened = try XCTUnwrap(PDFDocument(url: copy)?.page(at: slot))
            assertGeometry(reopened, matches: previous, normalized: true)
            let pdfPage = try XCTUnwrap(CGPDFDocument(copy as CFURL)?.page(at: slot + 1))
            let bitmap = try XCTUnwrap(CGContext(data: nil, width: 32, height: 32, bitsPerComponent: 8, bytesPerRow: 128,
                                                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
            bitmap.setFillColor(CGColor(gray: 1, alpha: 1)); bitmap.fill(CGRect(x: 0, y: 0, width: 32, height: 32))
            bitmap.concatenate(pdfPage.getDrawingTransform(.mediaBox, rect: CGRect(x: 0, y: 0, width: 32, height: 32), rotate: 0, preserveAspectRatio: true))
            bitmap.drawPDFPage(pdfPage)
            let bytes = try XCTUnwrap(bitmap.data).assumingMemoryBound(to: UInt8.self)
            for pixel in 0..<(32 * 32) { for channel in 0..<3 { XCTAssertEqual(bytes[pixel * 4 + channel], 255) } }
            workspace.undo(); XCTAssertEqual(workspace.document?.pageCount, 3)
            XCTAssertFalse(workspace.canUndo)
            workspace.redo(); XCTAssertEqual(workspace.document?.pageCount, 4)
            assertGeometry(try XCTUnwrap(workspace.document?.page(at: slot)), matches: previous, normalized: true)
        }
        XCTAssertEqual(try Data(contentsOf: source), original)
    }

    func testNewDocumentDefaultAndInvalidInsertionRemainUnchanged() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        XCTAssertEqual(workspace.document?.page(at: 0)?.bounds(for: .mediaBox).size, CGSize(width: 612, height: 792))
        let document = workspace.document, revision = workspace.revision
        for slot in [-1, 2] { XCTAssertThrowsError(try workspace.insertBlank(at: slot)) }
        XCTAssertTrue(workspace.document === document)
        XCTAssertEqual(workspace.revision, revision); XCTAssertFalse(workspace.canUndo)
    }

    private func assertGeometry(_ page: PDFPage, matches reference: PDFPage, normalized: Bool = false, file: StaticString = #filePath, line: UInt = #line) {
        for box in [PDFDisplayBox.mediaBox, .cropBox, .bleedBox, .trimBox, .artBox] {
            let actual = page.bounds(for: box), expected = reference.bounds(for: box)
            if normalized {
                // PDFKit may translate the media origin to zero when writing.
                let actualMedia = page.bounds(for: .mediaBox), expectedMedia = reference.bounds(for: .mediaBox)
                XCTAssertEqual(actual.size, expected.size, file: file, line: line)
                XCTAssertEqual(actual.origin.x - actualMedia.origin.x, expected.origin.x - expectedMedia.origin.x, file: file, line: line)
                XCTAssertEqual(actual.origin.y - actualMedia.origin.y, expected.origin.y - expectedMedia.origin.y, file: file, line: line)
            } else {
                XCTAssertEqual(actual, expected, file: file, line: line)
            }
        }
        XCTAssertEqual(page.rotation, reference.rotation, file: file, line: line)
    }
}
