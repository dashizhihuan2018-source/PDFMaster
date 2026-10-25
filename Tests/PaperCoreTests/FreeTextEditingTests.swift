import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor final class FreeTextEditingTests: XCTestCase {
    private let bounds = CGRect(x: 60, y: 250, width: 260, height: 100)

    private func blank() throws -> PDFWorkspace {
        let workspace = PDFWorkspace(); try workspace.createBlank(); return workspace
    }

    func testMultilineCreationUsesOneCommentingTransactionAndBackwardCompatibleDefaults() throws {
        let workspace = try blank()
        let text = "中文自由输入\nSecond line\n第三行\tTab"
        try workspace.addText(text, page: 0, bounds: bounds)
        let annotation = try XCTUnwrap(workspace.document?.page(at: 0)?.annotations.first)
        XCTAssertEqual(annotation.type, "FreeText")
        XCTAssertEqual(annotation.contents, text)
        XCTAssertEqual(annotation.font?.pointSize, 12)
        XCTAssertEqual(annotation.alignment, .left)
        XCTAssertEqual(annotation.color.alphaComponent, 0)
        XCTAssertEqual(workspace.revision, 1)
        XCTAssertEqual(workspace.undo(), .annotations(IndexSet(integer: 0)))
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.count, 0)
        XCTAssertFalse(workspace.canUndo)
        XCTAssertFalse(workspace.isDirty)
        XCTAssertEqual(workspace.redo(), .annotations(IndexSet(integer: 0)))
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.first?.contents, text)
    }

    func testStyleBoundsTextAndRotationSurviveSaveReopenAndUndoRedo() throws {
        for rotation in [0, 90, 180, 270] {
            let workspace = try blank()
            try workspace.addText("Original", page: 0, bounds: bounds)
            let editedBounds = CGRect(x: 120, y: 320, width: 300, height: 160)
            let font = try XCTUnwrap(NSFont(name: "Helvetica-Bold", size: 22))
            let color = NSColor(deviceRed: 0.2, green: 0.4, blue: 0.7, alpha: 1)
            let text = "自由文字\nEditable second line"
            try workspace.updateFreeText(page: 0, index: 0, text: text, bounds: editedBounds,
                                         font: font, color: color, rotation: rotation)
            XCTAssertEqual(workspace.revision, 2)
            XCTAssertEqual(workspace.undo(), .annotations(IndexSet(integer: 0)))
            XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.first?.contents, "Original")
            XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.first?.bounds, bounds)
            XCTAssertEqual(workspace.redo(), .annotations(IndexSet(integer: 0)))
            let output = FileManager.default.temporaryDirectory.appendingPathComponent("FreeText-\(UUID().uuidString).pdf")
            defer { try? FileManager.default.removeItem(at: output) }
            try workspace.saveCopy(to: output)
            let reopened = try XCTUnwrap(PDFDocument(url: output)?.page(at: 0)?.annotations.first)
            XCTAssertEqual(reopened.contents, text)
            XCTAssertEqual(reopened.bounds, editedBounds)
            XCTAssertEqual(reopened.font?.fontName, font.fontName)
            XCTAssertEqual(reopened.font?.pointSize ?? 0, 22, accuracy: 0.01)
            XCTAssertEqual((reopened.value(forAnnotationKey: PDFAnnotationKey(rawValue: "/Rotate")) as? NSNumber)?.intValue, rotation)
            let actualColor = try XCTUnwrap(reopened.fontColor?.usingColorSpace(.deviceRGB))
            XCTAssertEqual(actualColor.redComponent, 0.2, accuracy: 0.01)
            XCTAssertEqual(actualColor.greenComponent, 0.4, accuracy: 0.01)
            XCTAssertEqual(actualColor.blueComponent, 0.7, accuracy: 0.01)
        }
    }

    func testEmptyUpdateRemovesOnlySelectedFreeTextAndCanUndoInOneStep() throws {
        let workspace = try blank()
        try workspace.addText("Keep", page: 0, bounds: bounds)
        try workspace.addText("Delete", page: 0, bounds: CGRect(x: 70, y: 450, width: 240, height: 80))
        let revision = workspace.revision
        try workspace.updateFreeText(page: 0, index: 1, text: "", bounds: bounds,
                                     font: .systemFont(ofSize: 14), color: .black)
        XCTAssertEqual(workspace.revision, revision + 1)
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.map(\.contents), ["Keep"])
        XCTAssertEqual(workspace.undo(), .annotations(IndexSet(integer: 0)))
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.map(\.contents), ["Keep", "Delete"])
        XCTAssertEqual(workspace.redo(), .annotations(IndexSet(integer: 0)))
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.map(\.contents), ["Keep"])
    }

    func testInvalidGeometryTextFontColorRotationNeverMutateOrCreateHistory() throws {
        let workspace = try blank(), original = workspace.document
        let badBounds = [CGRect.zero,
                         CGRect(x: CGFloat.nan, y: 20, width: 80, height: 40),
                         CGRect(x: 20, y: 20, width: CGFloat.infinity, height: 40),
                         CGRect(x: -1, y: 20, width: 80, height: 40),
                         CGRect(x: 20, y: 20, width: -80, height: 40),
                         CGRect(x: 20, y: 20, width: 9999, height: 40)]
        for rectangle in badBounds {
            XCTAssertThrowsError(try workspace.addText("draft", page: 0, bounds: rectangle))
        }
        for text in ["", "bad\u{0}text", "bad\u{1b}text", String(repeating: "a", count: 100_001)] {
            XCTAssertThrowsError(try workspace.addText(text, page: 0, bounds: bounds))
        }
        for rotation in [-90, 1, 45, 360, Int.max] {
            XCTAssertThrowsError(try workspace.addText("draft", page: 0, bounds: bounds, rotation: rotation))
        }
        for size: CGFloat in [0.5, 17.5, 513] {
            XCTAssertThrowsError(try workspace.addText("draft", page: 0, bounds: bounds, font: .systemFont(ofSize: size)))
        }
        let pattern = NSColor(patternImage: NSImage(size: NSSize(width: 2, height: 2)))
        XCTAssertThrowsError(try workspace.addText("draft", page: 0, bounds: bounds, color: pattern))
        XCTAssertTrue(workspace.document === original)
        XCTAssertEqual(workspace.revision, 0)
        XCTAssertFalse(workspace.isDirty)
        XCTAssertFalse(workspace.canUndo)
    }

    func testUpdateRejectsWrongTypesInvalidIndicesAndPagesWithoutChangingHistory() throws {
        let workspace = try blank()
        try workspace.addShape(.rectangle, page: 0, bounds: bounds, style: .default)
        let original = workspace.document, revision = workspace.revision
        for (page, index) in [(0, 0), (0, -1), (0, 1), (-1, 0), (1, 0)] {
            XCTAssertThrowsError(try workspace.updateFreeText(page: page, index: index, text: "Changed", bounds: bounds,
                                                              font: .systemFont(ofSize: 14), color: .black))
            XCTAssertThrowsError(try workspace.updateFreeText(page: page, index: index, text: "", bounds: bounds,
                                                              font: .systemFont(ofSize: 14), color: .black))
        }
        XCTAssertTrue(workspace.document === original)
        XCTAssertEqual(workspace.revision, revision)
        XCTAssertEqual(workspace.undo(), .annotations(IndexSet(integer: 0)))
        XCTAssertFalse(workspace.canUndo)
    }

    func testNonzeroCropIsEnforcedForCreationAndUpdateAcrossPageRotations() throws {
        for angle in [0, 90, 180, 270] {
            let workspace = try blank()
            let crop = CGRect(x: 100, y: 100, width: 300, height: 400)
            try workspace.cropPage(at: 0, to: crop)
            if angle != 0 { try workspace.rotate(page: 0, degrees: angle) }
            try workspace.addText("Crop edge", page: 0, bounds: crop, rotation: angle)
            let original = workspace.document, revision = workspace.revision
            let overflow = crop.offsetBy(dx: 0.01, dy: 0)
            XCTAssertThrowsError(try workspace.addText("Outside", page: 0, bounds: overflow))
            XCTAssertThrowsError(try workspace.updateFreeText(page: 0, index: 0, text: "Outside", bounds: overflow,
                                                              font: .systemFont(ofSize: 14), color: .black))
            XCTAssertTrue(workspace.document === original)
            XCTAssertEqual(workspace.revision, revision)
        }
    }

    func testSavedCopyDoesNotModifyOriginalSource() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("FreeTextSource-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = directory.appendingPathComponent("source.pdf"), copy = directory.appendingPathComponent("copy.pdf")
        let initial = try blank(); try initial.save(to: source)
        let sourceBytes = try Data(contentsOf: source)
        let workspace = PDFWorkspace(); try workspace.open(source)
        try workspace.addText("新增自由文字\nSecond line", page: 0, bounds: bounds, font: .systemFont(ofSize: 18), color: .blue)
        try workspace.updateFreeText(page: 0, index: 0, text: "Updated\n中文", bounds: bounds,
                                     font: .boldSystemFont(ofSize: 20), color: .red)
        try workspace.saveCopy(to: copy)
        XCTAssertEqual(try Data(contentsOf: source), sourceBytes)
        XCTAssertEqual(PDFDocument(url: copy)?.page(at: 0)?.annotations.first?.contents, "Updated\n中文")
        XCTAssertEqual(PDFDocument(url: source)?.page(at: 0)?.annotations.count, 0)
    }

    func testInitialSystem14PointFontSurvivesUndoRedoAndSaveReopen() throws {
        let workspace = try blank()
        try workspace.addText("系统字体\n14pt text", page: 0, bounds: bounds,
                              font: .systemFont(ofSize: 14), color: .blue)
        _ = workspace.undo(); _ = workspace.redo()
        let output = FileManager.default.temporaryDirectory.appendingPathComponent("FreeText14-\(UUID().uuidString).pdf")
        defer { try? FileManager.default.removeItem(at: output) }
        try workspace.saveCopy(to: output)
        let annotation = try XCTUnwrap(PDFDocument(url: output)?.page(at: 0)?.annotations.first)
        XCTAssertEqual(annotation.font?.pointSize ?? 0, 14, accuracy: 0.01)
        XCTAssertEqual(annotation.contents, "系统字体\n14pt text")
        let color = try XCTUnwrap(annotation.fontColor?.usingColorSpace(.deviceRGB))
        XCTAssertGreaterThan(color.blueComponent, 0.9)
    }

    func testWriterRefreshKeepsPagesAnnotationsOutlinesAndMetadata() throws {
        let workspace = try blank(); try workspace.insertBlank(at: 1)
        let doc = try XCTUnwrap(workspace.document)
        doc.documentAttributes = [PDFDocumentAttribute.titleAttribute: "Preserve catalog title"]
        let root = PDFOutline(), bookmark = PDFOutline()
        bookmark.label = "Second page"
        bookmark.destination = PDFDestination(page: try XCTUnwrap(doc.page(at: 1)), at: CGPoint(x: 40, y: 50))
        root.insertChild(bookmark, at: 0); doc.outlineRoot = root
        try workspace.addText("First", page: 1, bounds: bounds)
        try workspace.addShape(.rectangle, page: 1, bounds: CGRect(x: 10, y: 10, width: 40, height: 40), style: .default)
        let original = try XCTUnwrap(workspace.document?.page(at: 1)?.annotations.first)
        original.userName = "Preserved author"
        original.setValue("existing-id", forAnnotationKey: PDFAnnotationKey(rawValue: "/NM"))
        try workspace.updateFreeText(page: 1, index: 0, text: "Updated\nSecond", bounds: bounds,
                                     font: .systemFont(ofSize: 18), color: .red)
        XCTAssertEqual(workspace.document?.pageCount, 2)
        XCTAssertEqual(workspace.document?.page(at: 1)?.annotations.map(\.type), ["FreeText", "Square"])
        XCTAssertEqual(workspace.document?.page(at: 1)?.annotations.first?.userName, "Preserved author")
        XCTAssertEqual(workspace.document?.page(at: 1)?.annotations.first?.value(forAnnotationKey: PDFAnnotationKey(rawValue: "/NM")) as? String, "existing-id")
        XCTAssertEqual(workspace.document?.documentAttributes?[PDFDocumentAttribute.titleAttribute] as? String, "Preserve catalog title")
        let output = FileManager.default.temporaryDirectory.appendingPathComponent("FreeTextMetadata-\(UUID().uuidString).pdf")
        defer { try? FileManager.default.removeItem(at: output) }
        try workspace.saveCopy(to: output)
        let saved = try XCTUnwrap(PDFDocument(url: output)), savedBookmark = try XCTUnwrap(saved.outlineRoot?.child(at: 0))
        XCTAssertEqual(savedBookmark.label, "Second page")
        XCTAssertEqual(saved.index(for: try XCTUnwrap(savedBookmark.destination?.page)), 1)
        XCTAssertEqual(saved.documentAttributes?[PDFDocumentAttribute.titleAttribute] as? String, "Preserve catalog title")
        XCTAssertEqual(saved.page(at: 1)?.annotations.first?.userName, "Preserved author")
        XCTAssertEqual(saved.page(at: 1)?.annotations.first?.font?.pointSize ?? 0, 18, accuracy: 0.01)
    }
}
