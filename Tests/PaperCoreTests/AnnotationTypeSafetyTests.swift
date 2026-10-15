import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor
final class AnnotationTypeSafetyTests: XCTestCase {
    private var temporaryDirectory: URL!

    override func setUpWithError() throws {
        temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("AnnotationTypeSafety-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: temporaryDirectory, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        if let temporaryDirectory { try? FileManager.default.removeItem(at: temporaryDirectory) }
    }

    private func fixture(_ subtype: PDFAnnotationSubtype) throws -> PDFWorkspace {
        let creator = PDFWorkspace()
        try creator.createBlank()
        let annotation = PDFAnnotation(bounds: CGRect(x: 40, y: 40, width: 100, height: 40),
                                       forType: subtype, withProperties: nil)
        annotation.contents = "Original"
        annotation.color = .red
        if subtype == .freeText { annotation.font = .systemFont(ofSize: 12) }
        if subtype == .line {
            annotation.startPoint = CGPoint(x: 10, y: 10)
            annotation.endPoint = CGPoint(x: 90, y: 30)
        }
        if subtype == .ink {
            let path = NSBezierPath()
            path.move(to: CGPoint(x: 10, y: 10))
            path.line(to: CGPoint(x: 90, y: 30))
            annotation.add(path)
        }
        if subtype == .highlight {
            annotation.quadrilateralPoints = [
                NSValue(point: CGPoint(x: 0, y: 40)), NSValue(point: CGPoint(x: 100, y: 40)),
                NSValue(point: .zero), NSValue(point: CGPoint(x: 100, y: 0))
            ]
        }
        if subtype == .widget {
            annotation.widgetFieldType = .text
            annotation.fieldName = "SelfGeneratedField"
            annotation.widgetStringValue = "Field value"
        }
        if subtype == .link { annotation.url = URL(string: "https://example.com/original") }
        creator.document?.page(at: 0)?.addAnnotation(annotation)
        let source = temporaryDirectory.appendingPathComponent("\(UUID().uuidString).pdf")
        try creator.saveCopy(to: source)
        let workspace = PDFWorkspace()
        try workspace.open(source)
        let reopened = try XCTUnwrap(workspace.document?.page(at: 0)?.annotations.first)
        XCTAssertEqual(reopened.type, String(subtype.rawValue.dropFirst()))
        return workspace
    }

    func testGenericEditRejectsPersistedWidgetsAndLinksWithoutChangingState() throws {
        for subtype: PDFAnnotationSubtype in [.widget, .link] {
            let workspace = try fixture(subtype)
            let source = try XCTUnwrap(workspace.sourceURL)
            let sourceBytes = try Data(contentsOf: source)
            let original = workspace.document
            let originalAnnotation = try XCTUnwrap(original?.page(at: 0)?.annotations.first)
            let originalBounds = originalAnnotation.bounds
            let originalContents = originalAnnotation.contents
            let originalColor = originalAnnotation.color
            let attempts: [(String?, CGRect?, NSColor?)] = [
                ("Changed", nil, nil),
                (nil, CGRect(x: 150, y: 150, width: 150, height: 50), nil),
                (nil, nil, .blue),
                (nil, nil, nil)
            ]
            for attempt in attempts {
                XCTAssertThrowsError(try workspace.editAnnotation(page: 0, index: 0,
                    contents: attempt.0, bounds: attempt.1, color: attempt.2)) {
                    XCTAssertEqual($0 as? PDFWorkspaceError, .invalidSelection)
                }
                XCTAssertTrue(workspace.document === original)
                XCTAssertEqual(workspace.revision, 0)
                XCTAssertFalse(workspace.isDirty)
                XCTAssertFalse(workspace.canUndo)
                XCTAssertFalse(workspace.canRedo)
                XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.first?.bounds, originalBounds)
                XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.first?.contents, originalContents)
                XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.first?.color, originalColor)
            }
            XCTAssertEqual(try Data(contentsOf: source), sourceBytes)
        }
    }

    func testGenericRemovalRejectsPersistedWidgetsAndLinksWithoutChangingState() throws {
        for subtype: PDFAnnotationSubtype in [.widget, .link] {
            let workspace = try fixture(subtype)
            let source = try XCTUnwrap(workspace.sourceURL)
            let sourceBytes = try Data(contentsOf: source)
            let original = workspace.document
            XCTAssertThrowsError(try workspace.removeAnnotation(page: 0, index: 0)) {
                XCTAssertEqual($0 as? PDFWorkspaceError, .invalidSelection)
            }
            XCTAssertTrue(workspace.document === original)
            XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.count, 1)
            XCTAssertEqual(workspace.revision, 0)
            XCTAssertFalse(workspace.isDirty)
            XCTAssertFalse(workspace.canUndo)
            XCTAssertFalse(workspace.canRedo)
            XCTAssertEqual(try Data(contentsOf: source), sourceBytes)
        }
    }

    func testProtectedTypeRejectionsPreservePendingRedo() throws {
        for subtype: PDFAnnotationSubtype in [.widget, .link] {
            let workspace = try fixture(subtype)
            try workspace.addText("Undoable ordinary annotation", page: 0,
                                  bounds: CGRect(x: 200, y: 200, width: 180, height: 40))
            workspace.undo()
            XCTAssertTrue(workspace.canRedo)
            let original = workspace.document
            let revision = workspace.revision
            XCTAssertThrowsError(try workspace.editAnnotation(page: 0, index: 0,
                contents: "Should not mutate", bounds: nil, color: nil))
            XCTAssertThrowsError(try workspace.removeAnnotation(page: 0, index: 0))
            XCTAssertTrue(workspace.document === original)
            XCTAssertEqual(workspace.revision, revision)
            XCTAssertFalse(workspace.isDirty)
            XCTAssertFalse(workspace.canUndo)
            XCTAssertTrue(workspace.canRedo)
            workspace.redo()
            XCTAssertTrue(workspace.document?.page(at: 0)?.annotations.contains {
                $0.type == "FreeText" && $0.contents == "Undoable ordinary annotation"
            } == true)
            XCTAssertTrue(workspace.document?.page(at: 0)?.annotations.contains { $0.type == String(subtype.rawValue.dropFirst()) } == true)
        }
    }

    func testOrdinaryAnnotationsRemainEditableRemovableAndUndoable() throws {
        for subtype: PDFAnnotationSubtype in [.text, .freeText, .square, .circle, .line, .ink, .highlight] {
            let workspace = try fixture(subtype)
            let originalBounds = try XCTUnwrap(workspace.document?.page(at: 0)?.annotations.first?.bounds)
            let changedBounds = CGRect(x: 180, y: 220, width: 140, height: 60)
            try workspace.editAnnotation(page: 0, index: 0, contents: "Edited", bounds: changedBounds, color: .blue)
            XCTAssertEqual(workspace.revision, 1)
            let persistedBounds = try XCTUnwrap(workspace.document?.page(at: 0)?.annotations.first?.bounds)
            // PDFKit normalizes a Text note to its 24-point icon and may add a
            // companion Popup. Other ordinary annotation bounds remain exact.
            if subtype != .text { XCTAssertEqual(persistedBounds, changedBounds) }
            XCTAssertNotEqual(persistedBounds, originalBounds)
            XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.first?.contents, "Edited")
            try workspace.removeAnnotation(page: 0, index: 0)
            XCTAssertEqual(workspace.revision, 2)
            let typeName = String(subtype.rawValue.dropFirst())
            XCTAssertFalse(workspace.document?.page(at: 0)?.annotations.contains { $0.type == typeName } == true)
            workspace.undo()
            XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.first?.bounds, persistedBounds)
            workspace.undo()
            XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.first?.bounds, originalBounds)
            XCTAssertFalse(workspace.isDirty)
            workspace.redo()
            workspace.redo()
            XCTAssertFalse(workspace.document?.page(at: 0)?.annotations.contains { $0.type == typeName } == true)
        }
    }
}
