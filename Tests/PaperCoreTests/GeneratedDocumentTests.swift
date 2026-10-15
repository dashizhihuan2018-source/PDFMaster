import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor final class GeneratedDocumentTests: XCTestCase {
    func testUnsavedGeneratedDocumentHasFreshDirtyStateAndNoBlankUndo() throws {
        let input = PDFWorkspace(); try input.createBlank()
        let data = try XCTUnwrap(input.document?.dataRepresentation())
        let target = PDFWorkspace(); try target.createUnsavedDocument(data: data)
        XCTAssertEqual(target.document?.pageCount, 1)
        XCTAssertTrue(target.isDirty); XCTAssertNil(target.sourceURL)
        XCTAssertFalse(target.canUndo); XCTAssertFalse(target.canRedo)
        XCTAssertEqual(target.revision, 0)
        XCTAssertTrue(target.allowsCommenting)
    }
    func testInvalidOrWidgetDocumentDoesNotReplaceExistingWorkspace() throws {
        let target = PDFWorkspace(); try target.createBlank()
        let original = target.document
        XCTAssertThrowsError(try target.createUnsavedDocument(data: Data("bad".utf8)))
        XCTAssertTrue(target.document === original); XCTAssertFalse(target.isDirty)
        let form = PDFWorkspace(); try form.createBlank()
        let widget = PDFAnnotation(bounds: CGRect(x: 10, y: 10, width: 30, height: 20), forType: .widget, withProperties: nil)
        form.document?.page(at: 0)?.addAnnotation(widget)
        let data = try XCTUnwrap(form.document?.dataRepresentation())
        XCTAssertThrowsError(try target.createUnsavedDocument(data: data))
        XCTAssertTrue(target.document === original); XCTAssertEqual(target.revision, 0)
    }
}
