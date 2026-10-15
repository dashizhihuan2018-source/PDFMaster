import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor
final class CanvasPDFAdmissionTests: XCTestCase {
    private func admit(_ bytes: Data, to workspace: PDFWorkspace) throws {
        try workspace.createUnsavedPDFCopy(data: bytes)
    }

    private func annotatedPDF() throws -> Data {
        let source = PDFWorkspace(); try source.createBlank()
        let page = try XCTUnwrap(source.document?.page(at: 0))
        let note = PDFAnnotation(bounds: CGRect(x: 40, y: 40, width: 24, height: 24), forType: .text, withProperties: nil)
        note.contents = "A local comment"
        page.addAnnotation(note)
        let popup = PDFAnnotation(bounds: CGRect(x: 80, y: 40, width: 160, height: 100), forType: .popup, withProperties: nil)
        popup.contents = "A retained popup"
        page.addAnnotation(popup)
        let ink = PDFAnnotation(bounds: CGRect(x: 100, y: 200, width: 100, height: 100), forType: .ink, withProperties: nil)
        let path = NSBezierPath(); path.move(to: CGPoint(x: 5, y: 5)); path.line(to: CGPoint(x: 70, y: 80)); ink.add(path)
        ink.color = .red; page.addAnnotation(ink)
        return try XCTUnwrap(source.document?.dataRepresentation())
    }

    func testRetainsCommentPopupAndInkInIndependentUnsavedCopy() throws {
        let data = try annotatedPDF()
        let workspace = PDFWorkspace(); try admit(data, to: workspace)
        let annotations = try XCTUnwrap(workspace.document?.page(at: 0)?.annotations)
        XCTAssertEqual(Set(annotations.compactMap(\.type)), Set(["Text", "Popup", "Ink"]))
        XCTAssertEqual(annotations.first { $0.type == "Text" }?.contents, "A local comment")
        XCTAssertFalse(try XCTUnwrap(annotations.first { $0.type == "Ink" }?.paths).isEmpty)
        XCTAssertNil(workspace.sourceURL); XCTAssertTrue(workspace.isDirty)
        XCTAssertFalse(workspace.isEncryptedSource); XCTAssertFalse(workspace.canUndo)
        XCTAssertEqual(workspace.revision, 0)
        let input = try workspace.engineInput()
        XCTAssertEqual(input.safetySourceData, data)
        XCTAssertEqual(input.safetySourceDatas, [data])
    }

    func testCopyEditingDoesNotMutateAssetBytesOrOriginalPDFDocument() throws {
        let data = try annotatedPDF(), untouchedData = data
        let original = try XCTUnwrap(PDFDocument(data: data))
        let workspace = PDFWorkspace(); try admit(data, to: workspace)
        try workspace.rotate(page: 0, degrees: 90)
        XCTAssertEqual(workspace.document?.page(at: 0)?.rotation, 90)
        XCTAssertEqual(original.page(at: 0)?.rotation, 0)
        XCTAssertEqual(data, untouchedData)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("CanvasCopyTests-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let sourceURL = directory.appendingPathComponent("original.pdf")
        try data.write(to: sourceURL)
        let output = directory.appendingPathComponent("edited-copy.pdf")
        try workspace.saveCopy(to: output)
        XCTAssertEqual(try Data(contentsOf: sourceURL), data)
        XCTAssertEqual(PDFDocument(url: output)?.page(at: 0)?.rotation, 90)
        let savedAnnotations = try XCTUnwrap(PDFDocument(url: output)?.page(at: 0)?.annotations)
        // PDFKit generates an associated Popup for Text annotations on write.
        // Verify the original payloads, not the implementation-specific popup count.
        XCTAssertEqual(savedAnnotations.filter { $0.type == "Text" }.map(\.contents), ["A local comment"])
        XCTAssertTrue(savedAnnotations.contains { $0.type == "Popup" && $0.contents == "A retained popup" })
        let savedInk = try XCTUnwrap(savedAnnotations.first { $0.type == "Ink" })
        XCTAssertFalse(try XCTUnwrap(savedInk.paths).isEmpty)
        XCTAssertEqual(Set(savedAnnotations.compactMap(\.type)), Set(["Text", "Popup", "Ink"]))
    }

    func testOldGeneratedOnlyAdmissionStillRejectsAnnotatedPDF() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        let original = workspace.document
        XCTAssertThrowsError(try workspace.createUnsavedDocument(data: annotatedPDF()))
        XCTAssertTrue(workspace.document === original)
        XCTAssertFalse(workspace.isDirty)
    }

    func testInvalidAndEncryptedAndWidgetInputsLeaveWorkspaceHistoryUntouched() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        try workspace.rotate(page: 0, degrees: 90); workspace.undo()
        let original = workspace.document, revision = workspace.revision
        let maker = PDFWorkspace(); try maker.createBlank()
        let encrypted = try XCTUnwrap(maker.document?.dataRepresentation(options: [PDFDocumentWriteOption.ownerPasswordOption: "owner", PDFDocumentWriteOption.userPasswordOption: "reader"]))
        let widget = PDFAnnotation(bounds: CGRect(x: 30, y: 30, width: 120, height: 30), forType: .widget, withProperties: nil)
        widget.widgetFieldType = .text; widget.fieldName = "LocalField"
        maker.document?.page(at: 0)?.addAnnotation(widget)
        let widgetData = try XCTUnwrap(maker.document?.dataRepresentation())
        for input in [Data("not a PDF".utf8), encrypted, widgetData, Data(repeating: 0, count: 50 * 1_024 * 1_024 + 1)] {
            XCTAssertThrowsError(try admit(input, to: workspace))
            XCTAssertTrue(workspace.document === original)
            XCTAssertEqual(workspace.revision, revision)
            XCTAssertFalse(workspace.isDirty); XCTAssertFalse(workspace.canUndo); XCTAssertTrue(workspace.canRedo)
        }
    }

    func testRejectsRawCatalogJavaScriptLaunchAndSignatureBeforeNormalization() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        let original = workspace.document
        let fixtures: [(String, String, String)] = [
            ("/OpenAction 4 0 R", "", "<< /S /JavaScript /JS (app.alert(1)) >>"),
            ("/OpenAction 4 0 R", "", "<< /S /Launch /F (external-tool) >>"),
            ("/AcroForm << /Fields [4 0 R] >>", "", "<< /FT /Sig /T (Signature1) >>"),
            ("/Names << /EmbeddedFiles 4 0 R >>", "", "<< /Names [] >>"),
            ("", "/Annots [4 0 R]", "<< /Type /Annot /Subtype /Widget /FT /Sig /Rect [0 0 20 20] /P 3 0 R >>"),
            ("", "/AA << /O 4 0 R >>", "<< /S /JavaScript /JS (app.alert(1)) >>")
        ]
        for fixture in fixtures {
            let data = minimalPDF(catalog: fixture.0, page: fixture.1, extra: fixture.2)
            XCTAssertNotNil(PDFDocument(data: data))
            XCTAssertThrowsError(try admit(data, to: workspace), fixture.0 + fixture.1)
            XCTAssertTrue(workspace.document === original)
        }
    }

    func testRejectsUnknownActiveAnnotationSubtype() throws {
        let data = minimalPDF(catalog: "", page: "/Annots [4 0 R]", extra: "<< /Type /Annot /Subtype /RichMedia /Rect [0 0 20 20] /P 3 0 R >>")
        XCTAssertNotNil(PDFDocument(data: data))
        XCTAssertThrowsError(try admit(data, to: PDFWorkspace()))
    }

    func testNewAdmissionInvalidatesOldEngineGenerationAndClearsHistory() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        let input = try workspace.engineInput()
        try workspace.rotate(page: 0, degrees: 90)
        try admit(annotatedPDF(), to: workspace)
        XCTAssertThrowsError(try workspace.validateEngineInput(input)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .staleEngineResult)
        }
        XCTAssertFalse(workspace.canUndo); XCTAssertFalse(workspace.canRedo)
        XCTAssertEqual(workspace.revision, 0)
    }

    /// Builds legal synthetic PDFs without any user documents or binary fixture.
    private func minimalPDF(catalog: String, page: String, extra: String) -> Data {
        let objects = ["<< /Type /Catalog /Pages 2 0 R \(catalog) >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \(page) >>", extra]
        var bytes = Data("%PDF-1.7\n".utf8), offsets: [Int] = []
        for (index, object) in objects.enumerated() {
            offsets.append(bytes.count)
            bytes.append(Data("\(index + 1) 0 obj\n\(object)\nendobj\n".utf8))
        }
        let start = bytes.count
        bytes.append(Data("xref\n0 5\n0000000000 65535 f \n".utf8))
        for offset in offsets { bytes.append(Data(String(format: "%010d 00000 n \n", offset).utf8)) }
        bytes.append(Data("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n\(start)\n%%EOF\n".utf8))
        return bytes
    }
}
