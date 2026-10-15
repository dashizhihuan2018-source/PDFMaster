import AppKit
import PDFKit
import ObjectiveC.runtime
import XCTest
@testable import PaperCore

private enum UndoSerializationCounter {
    static let lock = NSLock()
    static var value = 0
    static func increment() { lock.lock(); value += 1; lock.unlock() }
}
private extension PDFDocument {
    @objc dynamic func paperUndoCountedDataRepresentation() -> Data? {
        UndoSerializationCounter.increment()
        return paperUndoCountedDataRepresentation()
    }
}

@MainActor final class UndoScopeTests: XCTestCase {
    func testAnnotationsRecordExactPagesAndRedoRetainsScopeAcrossBranching() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank(); try workspace.insertBlank(at: 1)
        try workspace.addInk(points: [CGPoint(x: 30, y: 40), CGPoint(x: 100, y: 100)], page: 1, color: .red, width: 3)
        XCTAssertEqual(workspace.undo(), .annotations(IndexSet(integer: 1)))
        XCTAssertEqual(workspace.document?.page(at: 1)?.annotations.count, 0)
        XCTAssertEqual(workspace.redo(), .annotations(IndexSet(integer: 1)))
        XCTAssertEqual(workspace.document?.page(at: 1)?.annotations.count, 1)
        XCTAssertEqual(workspace.undo(), .annotations(IndexSet(integer: 1)))
        try workspace.addText("branch", page: 0, bounds: CGRect(x: 50, y: 50, width: 80, height: 20))
        XCTAssertFalse(workspace.canRedo)
        XCTAssertNil(workspace.redo())
        XCTAssertEqual(workspace.undo(), .annotations(IndexSet(integer: 0)))
        XCTAssertEqual(workspace.redo(), .annotations(IndexSet(integer: 0)))
    }

    func testPageGeometryNeverPretendsToBeAnnotationOnlyEvenWithEqualPageCount() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        try workspace.rotate(page: 0, degrees: 90)
        XCTAssertEqual(workspace.undo(), .document)
        XCTAssertEqual(workspace.document?.page(at: 0)?.rotation, 0)
        XCTAssertEqual(workspace.redo(), .document)
        XCTAssertEqual(workspace.document?.page(at: 0)?.rotation, 90)
        try workspace.insertBlank(at: 1)
        XCTAssertEqual(workspace.undo(), .document)
        XCTAssertEqual(workspace.document?.pageCount, 1)
        XCTAssertEqual(workspace.redo(), .document)
        XCTAssertEqual(workspace.document?.pageCount, 2)
    }

    func testNoHistoryLeavesDirtyRevisionAndDocumentUntouched() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        let document = workspace.document
        XCTAssertNil(workspace.undo()); XCTAssertNil(workspace.redo())
        XCTAssertTrue(workspace.document === document)
        XCTAssertFalse(workspace.isDirty); XCTAssertEqual(workspace.revision, 0)
        try workspace.addShape(.rectangle, page: 0, bounds: CGRect(x: 50, y: 50, width: 80, height: 60), style: .default)
        XCTAssertTrue(workspace.isDirty)
        XCTAssertEqual(workspace.undo(), .annotations(IndexSet(integer: 0)))
        XCTAssertFalse(workspace.isDirty)
        XCTAssertEqual(workspace.redo(), .annotations(IndexSet(integer: 0)))
        XCTAssertTrue(workspace.isDirty)
    }

    func testRepeatedInkEraseUndoRedoNeverConsumesAnotherStroke() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        for index in 0..<3 {
            try workspace.addInk(points: [CGPoint(x: 30, y: 40 + index * 30), CGPoint(x: 100, y: 100 + index * 30)], page: 0, color: .red, width: 3)
        }
        let initialBounds = try XCTUnwrap(workspace.document?.page(at: 0)).annotations.map(\.bounds)
        for _ in 0..<8 {
            try workspace.removeAnnotation(page: 0, index: 1)
            XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.map(\.bounds), [initialBounds[0], initialBounds[2]])
            XCTAssertEqual(workspace.undo(), .annotations(IndexSet(integer: 0)))
            XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.map(\.bounds), initialBounds)
            XCTAssertEqual(workspace.redo(), .annotations(IndexSet(integer: 0)))
            XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.count, 2)
            XCTAssertEqual(workspace.undo(), .annotations(IndexSet(integer: 0)))
            XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.map(\.bounds), initialBounds)
        }
    }

    func testNegativeCommentPageRejectsBeforeHistoryChanges() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        let document = workspace.document
        XCTAssertThrowsError(try workspace.addText("invalid", page: -1, bounds: CGRect(x: 10, y: 10, width: 80, height: 30)))
        XCTAssertThrowsError(try workspace.addInk(points: [CGPoint(x: 30, y: 40), CGPoint(x: 100, y: 100)], page: -1, color: .red, width: 3))
        XCTAssertTrue(workspace.document === document)
        XCTAssertFalse(workspace.canUndo); XCTAssertEqual(workspace.revision, 0)
    }

    func testEmptyHistoryDoesNotSerializeEntireDocument() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        let original = try XCTUnwrap(class_getInstanceMethod(PDFDocument.self, #selector(PDFDocument.dataRepresentation)))
        let counted = try XCTUnwrap(class_getInstanceMethod(PDFDocument.self, #selector(PDFDocument.paperUndoCountedDataRepresentation)))
        UndoSerializationCounter.value = 0
        method_exchangeImplementations(original, counted)
        defer { method_exchangeImplementations(original, counted) }
        XCTAssertNil(workspace.undo()); XCTAssertNil(workspace.redo())
        XCTAssertEqual(UndoSerializationCounter.value, 0)
    }
}
