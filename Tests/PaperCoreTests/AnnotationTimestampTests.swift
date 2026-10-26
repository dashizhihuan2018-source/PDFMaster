import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor final class AnnotationTimestampTests: XCTestCase {
    func testNewAnnotationsHavePersistedModificationTimes() throws {
        let workspace = PDFWorkspace()
        try workspace.createBlank()
        let earliest = Date().addingTimeInterval(-2)
        let box = CGRect(x: 40, y: 100, width: 120, height: 40)
        try workspace.addShape(.rectangle, page: 0, bounds: box, style: .default)
        try workspace.addLine(from: CGPoint(x: 40, y: 200), to: CGPoint(x: 180, y: 230), page: 0, style: .default, endings: .arrow)
        try workspace.addText("Timestamp", page: 0, bounds: box)
        try workspace.addInk(points: [CGPoint(x: 30, y: 300), CGPoint(x: 180, y: 320)], page: 0, color: .red, width: 2)
        try workspace.addInk(points: [CGPoint(x: 30, y: 350), CGPoint(x: 180, y: 380)], page: 0, color: .red, width: 2, smoothed: true)
        try workspace.addPopupNote("Recorded note", author: nil, page: 0, bounds: box, color: .yellow)
        let bytes = try XCTUnwrap(workspace.document?.dataRepresentation())
        let reopened = try XCTUnwrap(PDFDocument(data: bytes))
        let annotations = try XCTUnwrap(reopened.page(at: 0)).annotations.filter { $0.type != "Popup" }
        XCTAssertEqual(annotations.count, 6)
        for annotation in annotations {
            let date = try XCTUnwrap(annotation.modificationDate, "Missing time for \(annotation.type ?? "unknown")")
            XCTAssertGreaterThanOrEqual(date, earliest)
            XCTAssertLessThanOrEqual(date, Date().addingTimeInterval(2))
        }
    }

    func testEditingOnlyChangesTargetTimeAndUndoRestoresPreviousTime() throws {
        let workspace = PDFWorkspace()
        try workspace.createBlank()
        let box = CGRect(x: 40, y: 100, width: 120, height: 40)
        let old = Date(timeIntervalSince1970: 100)
        for _ in 0..<2 {
            let annotation = PDFAnnotation(bounds: box, forType: .square, withProperties: nil)
            annotation.modificationDate = old
            workspace.document?.page(at: 0)?.addAnnotation(annotation)
        }
        try workspace.updateShape(page: 0, index: 0, bounds: box, style: .default)
        XCTAssertGreaterThan(try XCTUnwrap(workspace.document?.page(at: 0)?.annotations[0].modificationDate), old)
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations[1].modificationDate, old)
        try workspace.undo()
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations[0].modificationDate, old)
    }

    func testTextInkAndNoteEditsRefreshSavedTime() throws {
        for type in ["FreeText", "Ink", "Text"] {
            let workspace = PDFWorkspace()
            try workspace.createBlank()
            let box = CGRect(x: 40, y: 100, width: 120, height: 40)
            if type == "FreeText" { try workspace.addText("Old", page: 0, bounds: box) }
            if type == "Ink" { try workspace.addInk(points: [.init(x: 40, y: 100), .init(x: 120, y: 120)], page: 0, color: .red, width: 2) }
            if type == "Text" { try workspace.addPopupNote("Old", author: nil, page: 0, bounds: box, color: .yellow) }
            let page = try XCTUnwrap(workspace.document?.page(at: 0))
            let index = try XCTUnwrap(page.annotations.firstIndex { $0.type == type })
            page.annotations[index].modificationDate = Date(timeIntervalSince1970: 100)
            let earliest = Date().addingTimeInterval(-2)
            if type == "FreeText" { try workspace.updateFreeText(page: 0, index: index, text: "Edited", bounds: box, font: .systemFont(ofSize: 12), color: .black) }
            if type == "Ink" { try workspace.updateInkStyle(page: 0, index: index, style: .default) }
            if type == "Text" { try workspace.editAnnotation(page: 0, index: index, contents: "Edited", bounds: nil, color: nil) }
            let saved = try XCTUnwrap(workspace.document?.dataRepresentation())
            let date = try XCTUnwrap(PDFDocument(data: saved)?.page(at: 0)?.annotations.first { $0.type == type }?.modificationDate)
            XCTAssertGreaterThanOrEqual(date, earliest, type)
        }
    }
}
