import AppKit
import CoreText
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor final class ExternalPageClipboardTests: XCTestCase {
    func testPageCopyOffersRealPNGAndTIFFWithoutLosingPDFPages() throws {
        let pb = NSPasteboard.withUniqueName(); defer { pb.releaseGlobally() }
        let c = WorkbenchController(); _ = c.view
        c.clipboard = DocumentClipboard(pasteboard: pb)
        try c.workspace.createBlank(); try c.workspace.insertBlank(at: 1)
        try c.workspace.addShape(.rectangle, page: 0, bounds: CGRect(x: 20, y: 40, width: 200, height: 100), style: .init(strokeColor: .red, fillColor: .red, lineWidth: 2))
        try c.workspace.addShape(.rectangle, page: 1, bounds: CGRect(x: 80, y: 100, width: 180, height: 120), style: .init(strokeColor: .blue, fillColor: .blue, lineWidth: 2))
        c.workspace.document?.page(at: 1)?.rotation = 90
        c.refreshDocument(); c.enterMode(.pages)
        c.pageOverview.restoreSelection(IndexSet([0, 1]))
        try c.performOverviewAction("copy-page")
        XCTAssertEqual(try XCTUnwrap(PDFDocument(data: XCTUnwrap(c.clipboard.pages))).pageCount, 2)
        XCTAssertEqual(pb.data(forType: .pdf), c.clipboard.pages)
        for type in [NSPasteboard.PasteboardType.png, .tiff] {
            let data = try XCTUnwrap(pb.data(forType: type), "Explicit image flavor required by external receivers")
            let bitmap = try XCTUnwrap(NSBitmapImageRep(data: data))
            XCTAssertGreaterThan(bitmap.pixelsHigh, 1000)
            XCTAssertLessThanOrEqual(bitmap.pixelsWide * bitmap.pixelsHigh, 16_777_216)
            XCTAssertLessThanOrEqual(max(bitmap.pixelsWide, bitmap.pixelsHigh), 8192)
            XCTAssertTrue(stride(from: 0, to: bitmap.pixelsHigh, by: 10).contains { y in
                stride(from: 0, to: bitmap.pixelsWide, by: 10).contains { x in
                    guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { return false }
                    return color.redComponent > 0.8 && color.greenComponent < 0.3 && color.blueComponent < 0.3
                }
            }, "Image must contain annotation pixels, not an empty white page")
            XCTAssertTrue(stride(from: bitmap.pixelsHigh / 2, to: bitmap.pixelsHigh, by: 10).contains { y in
                stride(from: 0, to: bitmap.pixelsWide, by: 10).contains { x in
                    guard let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB) else { return false }
                    return color.blueComponent > 0.8 && color.redComponent < 0.3 && color.greenComponent < 0.3
                }
            }, "The rotated second page must be present in the lower half")
        }
    }

    func testReaderCopyWithoutSelectionCopiesCurrentPageAsPDFAndImage() throws {
        let pb = NSPasteboard.withUniqueName(); defer { pb.releaseGlobally() }
        let c = WorkbenchController(); _ = c.view; c.clipboard = DocumentClipboard(pasteboard: pb)
        try c.workspace.createBlank(); c.refreshDocument()
        guard c.pdfView.validateMenuItem(NSMenuItem(title: "复制", action: #selector(NSText.copy(_:)), keyEquivalent: "c")) else {
            XCTFail("Current page copy should be available without a text selection")
            return
        }
        c.pdfView.copy(nil)
        XCTAssertNotNil(c.clipboard.pages)
        XCTAssertNotNil(pb.data(forType: .png))
        XCTAssertNil(c.clipboard.annotation)
    }

    func testAnnotationCopyKeepsObjectPriorityOverPageFallback() throws {
        let pb = NSPasteboard.withUniqueName(); defer { pb.releaseGlobally() }
        let c = WorkbenchController(); _ = c.view; c.clipboard = DocumentClipboard(pasteboard: pb)
        try c.workspace.createBlank()
        try c.workspace.addShape(.rectangle, page: 0, bounds: CGRect(x: 20, y: 40, width: 30, height: 30), style: .init(strokeColor: .red, fillColor: nil, lineWidth: 2))
        c.refreshDocument(); c.pdfView.onAnnotationSelection?(0, 0)
        c.pdfView.copy(nil)
        XCTAssertNotNil(c.clipboard.annotation); XCTAssertNil(c.clipboard.pages)
        pb.clearContents(); pb.setString("external replacement", forType: .string)
        XCTAssertFalse(c.clipboard.hasPages); XCTAssertFalse(c.clipboard.hasAnnotation)
    }

    func testInvalidPageDataLeavesExistingClipboardUntouched() {
        let pb = NSPasteboard.withUniqueName(); defer { pb.releaseGlobally() }
        pb.setString("existing clipboard", forType: .string)
        XCTAssertFalse(DocumentClipboard(pasteboard: pb).write(Data([1, 2, 3]), type: DocumentClipboard.pagesType))
        XCTAssertEqual(pb.string(forType: .string), "existing clipboard")
    }

    func testEscapeClearsInvisibleAnnotationSelectionBeforeCopyingPage() throws {
        let pb = NSPasteboard.withUniqueName(); defer { pb.releaseGlobally() }
        let c = WorkbenchController(); _ = c.view; c.clipboard = DocumentClipboard(pasteboard: pb)
        try c.workspace.createBlank()
        try c.workspace.addShape(.rectangle, page: 0, bounds: CGRect(x: 20, y: 40, width: 30, height: 30), style: .init(strokeColor: .red, fillColor: nil, lineWidth: 2))
        c.refreshDocument(); c.pdfView.onAnnotationSelection?(0, 0)
        XCTAssertTrue(c.pdfView.cancelPlacementForEscape())
        c.pdfView.copy(nil)
        XCTAssertNil(c.clipboard.annotation)
        XCTAssertNotNil(c.clipboard.pages)
        XCTAssertNotNil(pb.data(forType: .png))
    }

    func testTextSelectionStaysWithNativeCopyInsteadOfBecomingPageImage() throws {
        let c = WorkbenchController(); _ = c.view
        let data = NSMutableData(); var box = CGRect(x: 0, y: 0, width: 300, height: 400)
        let context = try XCTUnwrap(CGContext(consumer: CGDataConsumer(data: data)!, mediaBox: &box, nil))
        context.beginPDFPage(nil)
        let line = CTLineCreateWithAttributedString(NSAttributedString(string: "Select this text", attributes: [.font: NSFont.systemFont(ofSize: 20)]))
        context.textPosition = CGPoint(x: 20, y: 200); CTLineDraw(line, context)
        context.endPDFPage(); context.closePDF()
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("clipboard-text-\(UUID()).pdf")
        try (data as Data).write(to: file); defer { try? FileManager.default.removeItem(at: file) }
        XCTAssertTrue(c.open(url: file))
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        let selection = try XCTUnwrap(page.selection(for: NSRange(location: 0, length: 6)))
        c.pdfView.setCurrentSelection(selection, animate: false)
        XCTAssertEqual(c.pdfView.onClipboard?(.copy), false)
    }
}
