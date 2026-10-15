import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor
final class ShapeEditingTests: XCTestCase {
    private func blankWorkspace() throws -> PDFWorkspace {
        let workspace = PDFWorkspace()
        try workspace.createBlank()
        return workspace
    }

    func testCreationRejectsUnboundedWidthsWithoutMutating() throws {
        let workspace = try blankWorkspace()
        let original = workspace.document
        for width: CGFloat in [0, 0.1, 32.1, .infinity, .nan] {
            let style = AnnotationStyle(strokeColor: .black, fillColor: nil, lineWidth: width)
            XCTAssertThrowsError(try workspace.addShape(.rectangle, page: 0,
                bounds: CGRect(x: 40, y: 40, width: 80, height: 60), style: style))
            XCTAssertThrowsError(try workspace.addLine(from: CGPoint(x: 40, y: 40),
                to: CGPoint(x: 120, y: 100), page: 0, style: style, endings: .arrow))
        }
        XCTAssertTrue(workspace.document === original)
        XCTAssertEqual(workspace.revision, 0)
        XCTAssertFalse(workspace.isDirty)
        XCTAssertFalse(workspace.canUndo)
    }

    func testCreationRequiresGeometryInsideCropBox() throws {
        let workspace = try blankWorkspace()
        try workspace.cropPage(at: 0, to: CGRect(x: 100, y: 100, width: 300, height: 400))
        let original = workspace.document
        let revision = workspace.revision
        XCTAssertThrowsError(try workspace.addShape(.ellipse, page: 0,
            bounds: CGRect(x: 90, y: 120, width: 40, height: 50), style: .default))
        XCTAssertThrowsError(try workspace.addLine(from: CGPoint(x: 80, y: 150),
            to: CGPoint(x: 200, y: 250), page: 0, style: .default, endings: .none))
        XCTAssertTrue(workspace.document === original)
        XCTAssertEqual(workspace.revision, revision)
    }

    func testLineCreationAllowsCropEdgesAndHorizontalVerticalDirections() throws {
        let workspace = try blankWorkspace()
        let crop = CGRect(x: 100, y: 100, width: 300, height: 400)
        try workspace.cropPage(at: 0, to: crop)
        for points in [(CGPoint(x: 100, y: 500), CGPoint(x: 400, y: 500)),
                       (CGPoint(x: 400, y: 100), CGPoint(x: 400, y: 500))] {
            try workspace.addLine(from: points.0, to: points.1, page: 0,
                style: AnnotationStyle(strokeColor: .black, fillColor: nil, lineWidth: 32), endings: .arrow)
            let annotation = try XCTUnwrap(workspace.document?.page(at: 0)?.annotations.last)
            XCTAssertTrue(crop.contains(annotation.bounds))
            XCTAssertEqual(annotation.startPoint.x + annotation.bounds.minX, points.0.x, accuracy: 0.001)
            XCTAssertEqual(annotation.startPoint.y + annotation.bounds.minY, points.0.y, accuracy: 0.001)
            XCTAssertEqual(annotation.endPoint.x + annotation.bounds.minX, points.1.x, accuracy: 0.001)
            XCTAssertEqual(annotation.endPoint.y + annotation.bounds.minY, points.1.y, accuracy: 0.001)
        }
    }

    func testSupportedWidthLimitsAreAcceptedAndImportedLineEndingsRemainUnchanged() throws {
        let workspace = try blankWorkspace()
        try workspace.addShape(.rectangle, page: 0, bounds: CGRect(x: 40, y: 40, width: 80, height: 60),
            style: AnnotationStyle(strokeColor: .black, fillColor: nil, lineWidth: 0.25))
        let line = PDFAnnotation(bounds: CGRect(x: 50, y: 150, width: 160, height: 100), forType: .line, withProperties: nil)
        line.startPoint = CGPoint(x: 10, y: 10)
        line.endPoint = CGPoint(x: 150, y: 90)
        line.startLineStyle = .diamond
        line.endLineStyle = .closedArrow
        let border = PDFBorder()
        border.lineWidth = 1
        border.style = .dashed
        border.dashPattern = [3, 2]
        line.border = border
        workspace.document?.page(at: 0)?.addAnnotation(line)
        try workspace.updateShape(page: 0, index: 1, bounds: CGRect(x: 250, y: 350, width: 160, height: 100),
            style: AnnotationStyle(strokeColor: .blue, fillColor: nil, lineWidth: 32))
        let updated = try XCTUnwrap(workspace.document?.page(at: 0)?.annotations.last)
        XCTAssertEqual(updated.startPoint, CGPoint(x: 10, y: 10))
        XCTAssertEqual(updated.endPoint, CGPoint(x: 150, y: 90))
        XCTAssertEqual(updated.startLineStyle, .diamond)
        XCTAssertEqual(updated.endLineStyle, .closedArrow)
        XCTAssertEqual(updated.border?.style, .dashed)
        XCTAssertEqual(updated.border?.lineWidth, 32)
    }

    func testShapeMoveResizeAndStylePersistThroughSaveReopen() throws {
        for shape in [AnnotationShape.rectangle, .ellipse] {
            let workspace = try blankWorkspace()
            try workspace.addShape(shape, page: 0,
                bounds: CGRect(x: 40, y: 40, width: 80, height: 60), style: .default)
            let moved = CGRect(x: 200, y: 220, width: 160, height: 90)
            let style = AnnotationStyle(strokeColor: .red, fillColor: .yellow, lineWidth: 7.5)
            try workspace.updateShape(page: 0, index: 0, bounds: moved, style: style)
            let output = FileManager.default.temporaryDirectory.appendingPathComponent("ShapeEditing-\(UUID().uuidString).pdf")
            defer { try? FileManager.default.removeItem(at: output) }
            try workspace.saveCopy(to: output)
            let annotation = try XCTUnwrap(PDFDocument(url: output)?.page(at: 0)?.annotations.first)
            XCTAssertEqual(annotation.type, shape == .rectangle ? "Square" : "Circle")
            XCTAssertEqual(annotation.bounds, moved)
            XCTAssertEqual(annotation.border?.lineWidth ?? 0, 7.5, accuracy: 0.001)
            assertColor(annotation.color, matches: .red)
            assertColor(try XCTUnwrap(annotation.interiorColor), matches: .yellow)
            try workspace.updateShape(page: 0, index: 0, bounds: moved, style: .default)
            try workspace.saveCopy(to: output)
            XCTAssertNil(PDFDocument(url: output)?.page(at: 0)?.annotations.first?.interiorColor)
        }
    }

    func testLineResizeScalesLocalEndpointsPreservingDirectionAndEndings() throws {
        let workspace = try blankWorkspace()
        try workspace.addLine(from: CGPoint(x: 180, y: 50), to: CGPoint(x: 60, y: 130),
            page: 0, style: .default, endings: .doubleArrow)
        let original = try XCTUnwrap(workspace.document?.page(at: 0)?.annotations.first)
        let beforeBounds = original.bounds
        let beforeStart = original.startPoint
        let beforeEnd = original.endPoint
        let resized = CGRect(x: 210, y: 310, width: beforeBounds.width * 2, height: beforeBounds.height * 0.5)
        try workspace.updateShape(page: 0, index: 0, bounds: resized,
            style: AnnotationStyle(strokeColor: .blue, fillColor: nil, lineWidth: 4))
        let output = FileManager.default.temporaryDirectory.appendingPathComponent("LineEditing-\(UUID().uuidString).pdf")
        defer { try? FileManager.default.removeItem(at: output) }
        try workspace.saveCopy(to: output)
        let annotation = try XCTUnwrap(PDFDocument(url: output)?.page(at: 0)?.annotations.first)
        XCTAssertEqual(annotation.bounds, resized)
        XCTAssertEqual(annotation.startPoint.x, beforeStart.x * 2, accuracy: 0.001)
        XCTAssertEqual(annotation.startPoint.y, beforeStart.y * 0.5, accuracy: 0.001)
        XCTAssertEqual(annotation.endPoint.x, beforeEnd.x * 2, accuracy: 0.001)
        XCTAssertEqual(annotation.endPoint.y, beforeEnd.y * 0.5, accuracy: 0.001)
        XCTAssertEqual(annotation.startLineStyle, .openArrow)
        XCTAssertEqual(annotation.endLineStyle, .openArrow)
        XCTAssertEqual(annotation.border?.lineWidth ?? 0, 4, accuracy: 0.001)
    }

    func testUpdateShapeIsExactlyOneUndoTransaction() throws {
        let workspace = try blankWorkspace()
        let initial = CGRect(x: 40, y: 40, width: 80, height: 60)
        try workspace.addShape(.rectangle, page: 0, bounds: initial, style: .default)
        let updated = CGRect(x: 80, y: 100, width: 140, height: 80)
        let revision = workspace.revision
        try workspace.updateShape(page: 0, index: 0, bounds: updated,
            style: AnnotationStyle(strokeColor: .red, fillColor: .yellow, lineWidth: 5))
        XCTAssertEqual(workspace.revision, revision + 1)
        workspace.undo()
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.first?.bounds, initial)
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.first?.border?.lineWidth, 1)
        workspace.redo()
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.first?.bounds, updated)
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.first?.border?.lineWidth, 5)
        workspace.undo()
        workspace.undo()
        XCTAssertTrue(workspace.document?.page(at: 0)?.annotations.isEmpty == true)
        XCTAssertFalse(workspace.isDirty)
    }

    func testInvalidUpdateLeavesDocumentHistoryAndRevisionUnchanged() throws {
        let workspace = try blankWorkspace()
        try workspace.addShape(.rectangle, page: 0,
            bounds: CGRect(x: 40, y: 40, width: 80, height: 60), style: .default)
        workspace.undo()
        workspace.redo()
        let original = workspace.document
        let revision = workspace.revision
        let bounds = CGRect(x: 40, y: 40, width: 80, height: 60)
        for badBounds in [CGRect(x: -1, y: 40, width: 80, height: 60),
                          CGRect(x: 40, y: 40, width: 0, height: 60),
                          CGRect(x: CGFloat.nan, y: 40, width: 80, height: 60),
                          CGRect(x: 600, y: 40, width: 80, height: 60)] {
            XCTAssertThrowsError(try workspace.updateShape(page: 0, index: 0, bounds: badBounds, style: .default))
        }
        for width: CGFloat in [0.1, 33, .infinity, .nan] {
            XCTAssertThrowsError(try workspace.updateShape(page: 0, index: 0, bounds: bounds,
                style: AnnotationStyle(strokeColor: .black, fillColor: nil, lineWidth: width)))
        }
        XCTAssertThrowsError(try workspace.updateShape(page: 2, index: 0, bounds: bounds, style: .default))
        XCTAssertThrowsError(try workspace.updateShape(page: 0, index: -1, bounds: bounds, style: .default))
        XCTAssertThrowsError(try workspace.updateShape(page: 0, index: 50, bounds: bounds, style: .default))
        XCTAssertTrue(workspace.document === original)
        XCTAssertEqual(workspace.revision, revision)
        XCTAssertTrue(workspace.canUndo)
        XCTAssertFalse(workspace.canRedo)
    }

    func testUpdateShapeRejectsWidgetLinkAndNonShapeAnnotations() throws {
        for subtype: PDFAnnotationSubtype in [.widget, .link, .freeText, .text, .ink, .highlight] {
            let workspace = try blankWorkspace()
            let bounds = CGRect(x: 40, y: 40, width: 80, height: 60)
            let annotation = PDFAnnotation(bounds: bounds, forType: subtype, withProperties: nil)
            workspace.document?.page(at: 0)?.addAnnotation(annotation)
            let original = workspace.document
            XCTAssertThrowsError(try workspace.updateShape(page: 0, index: 0, bounds: bounds, style: .default))
            XCTAssertTrue(workspace.document === original)
            XCTAssertEqual(workspace.revision, 0)
            XCTAssertFalse(workspace.isDirty)
            XCTAssertFalse(workspace.canUndo)
        }
    }

    private func assertColor(_ color: NSColor, matches expected: NSColor, file: StaticString = #filePath, line: UInt = #line) {
        let actual = color.usingColorSpace(.deviceRGB)!
        let expected = expected.usingColorSpace(.deviceRGB)!
        XCTAssertEqual(actual.redComponent, expected.redComponent, accuracy: 0.001, file: file, line: line)
        XCTAssertEqual(actual.greenComponent, expected.greenComponent, accuracy: 0.001, file: file, line: line)
        XCTAssertEqual(actual.blueComponent, expected.blueComponent, accuracy: 0.001, file: file, line: line)
    }
}
