import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor final class PDFCopilotContextTests: XCTestCase {
    private func document() -> PDFDocument {
        let data = NSMutableData(); var box = CGRect(x: 0, y: 0, width: 300, height: 300)
        let context = CGContext(consumer: CGDataConsumer(data: data)!, mediaBox: &box, nil)!
        for text in ["First page", "Second page"] {
            context.beginPDFPage(nil); NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: false)
            (text as NSString).draw(at: CGPoint(x: 20, y: 200), withAttributes: [.font: NSFont.systemFont(ofSize: 18)])
            NSGraphicsContext.restoreGraphicsState(); context.endPDFPage()
        }
        context.closePDF(); return PDFDocument(data: data as Data)!
    }
    func testCurrentPageContainsOnlySelectedPageWithMarker() throws {
        let result = try PDFCopilotContext.extract(document: document(), scope: .currentPage, currentPage: 1)
        XCTAssertEqual(result.pages, [2]); XCTAssertTrue(result.text.contains("[p.2]"))
        XCTAssertTrue(result.text.contains("Second page")); XCTAssertFalse(result.text.contains("First page"))
    }
    func testWholeDocumentBudgetIsHardCapAndReportsTruncation() throws {
        let result = try PDFCopilotContext.extract(document: document(), scope: .wholeDocument, currentPage: 0, characterLimit: 20)
        XCTAssertLessThanOrEqual(result.text.count, 20); XCTAssertTrue(result.isTruncated)
    }
    func testCopyRestrictionAndEmptyDocumentsAreRefused() throws {
        let doc = document()
        let data = try XCTUnwrap(doc.dataRepresentation(options: [PDFDocumentWriteOption.ownerPasswordOption: "owner-only", PDFDocumentWriteOption.userPasswordOption: "open", PDFDocumentWriteOption.accessPermissionsOption: 0]))
        let protected = try XCTUnwrap(PDFDocument(data: data)); XCTAssertTrue(protected.unlock(withPassword: "open"))
        XCTAssertThrowsError(try PDFCopilotContext.extract(document: protected, scope: .wholeDocument, currentPage: 0))
        XCTAssertThrowsError(try PDFCopilotContext.extract(document: PDFDocument(), scope: .currentPage, currentPage: 0))
    }
    func testAnnotationSummaryIncludesOnlyNotesInScope() throws {
        let doc = document(); let note = PDFAnnotation(bounds: CGRect(x: 10, y: 10, width: 20, height: 20), forType: .text, withProperties: nil)
        note.contents = "Review later"; doc.page(at: 1)?.addAnnotation(note)
        let result = try PDFCopilotContext.extract(document: doc, scope: .currentPage, currentPage: 1, annotationsOnly: true)
        XCTAssertTrue(result.text.contains("Review later")); XCTAssertFalse(result.text.contains("Second page"))
    }
    func testActionsAndBoundedCitations() throws {
        let context = try PDFCopilotContext.extract(document: document(), scope: .wholeDocument, currentPage: 0)
        for action in PDFCopilotAction.allCases {
            let messages = context.messages(action: action, question: "Explain this", language: "English")
            XCTAssertEqual(messages.first?.role, "system"); XCTAssertTrue(messages.last!.content.contains(context.text))
        }
        XCTAssertEqual(context.citedPages(in: "See [p.2] [p.999] [p.0] [p.1] [p.2]"), [2, 1])
    }
}
