import XCTest
import AppKit
import PDFKit
import PaperCore
@testable import PaperDesk

@MainActor final class ContentEditIntegrationTests: XCTestCase {
    private lazy var publicFixture = try! PublicContentFixture()
    private var fixture: URL { publicFixture.source }
    private var engine: ContentEditEngine { ContentEditEngine() }

    private func controller() throws -> WorkbenchController {
        XCTAssertTrue(FileManager.default.fileExists(atPath: fixture.path))
        let c = WorkbenchController(); _ = c.view; try c.workspace.open(fixture); c.refreshDocument(); return c
    }
    private func inspect(_ input: PDFEngineInput) throws -> ContentPage {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-content-test-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: dir) }
        let url = dir.appendingPathComponent("input.pdf"); try input.data.write(to: url)
        return try engine.inspect(url, page: 0, cancelled: { false })
    }

    func testActualOriginalTextReplacementAndCJKUndoRedo() async throws {
        let original = try Data(contentsOf: fixture)
        let c = try controller()
        var input = try c.workspace.engineInput()
        var page = try inspect(input)
        let target = try XCTUnwrap(page.spans.first { $0.text == "Original target" })
        var request = ContentEditRequest(operation: "replace-text", page: 0); request.id = target.id; request.text = "Edited target"
        try await c.applyContentEdit(request, input: input, engine: engine)
        XCTAssertTrue(c.pdfView.document!.page(at: 0)!.string!.contains("Edited target"))
        XCTAssertFalse(c.pdfView.document!.page(at: 0)!.string!.contains("Original target"))
        XCTAssertTrue(c.pdfView.document!.page(at: 0)!.string!.contains("Neighbor preserved"))
        c.undo(); XCTAssertTrue(c.pdfView.document!.page(at: 0)!.string!.contains("Original target"))
        c.redo(); XCTAssertTrue(c.pdfView.document!.page(at: 0)!.string!.contains("Edited target"))
        input = try c.workspace.engineInput(); page = try inspect(input)
        request.id = try XCTUnwrap(page.spans.first { $0.text == "中文原文" }).id; request.text = "修改内容"
        try await c.applyContentEdit(request, input: input, engine: engine)
        XCTAssertTrue(c.pdfView.document!.page(at: 0)!.string!.contains("修改内容"))
        XCTAssertFalse(c.pdfView.document!.page(at: 0)!.string!.contains("中文原文"))
        XCTAssertEqual(try Data(contentsOf: fixture), original)
        XCTAssertFalse(LocalToolActivity.hasRunningJobs)
        XCTAssertEqual(ToolCatalog.descriptor(id: "edit")?.availability, .partial)
    }

    func testImageAndLinkObjectsActuallyWriteIntoDocument() async throws {
        let c = try controller(); var input = try c.workspace.engineInput(); var page = try inspect(input)
        var request = ContentEditRequest(operation: "replace-image", page: 0)
        request.id = try XCTUnwrap(page.images.first).id
        request.image_path = publicFixture.replacement.path
        try await c.applyContentEdit(request, input: input, engine: engine)
        XCTAssertTrue(c.workspace.isDirty)
        input = try c.workspace.engineInput()
        request = ContentEditRequest(operation: "insert-link", page: 0); request.bbox = [30, 340, 180, 370]; request.uri = "https://example.com/qa"
        try await c.applyContentEdit(request, input: input, engine: engine)
        XCTAssertTrue(c.workspace.document!.page(at: 0)!.annotations.contains { $0.type == "Link" && $0.url?.absoluteString == "https://example.com/qa" })
        input = try c.workspace.engineInput(); page = try inspect(input)
        request = ContentEditRequest(operation: "delete-link", page: 0); request.id = try XCTUnwrap(page.links.first).id
        try await c.applyContentEdit(request, input: input, engine: engine)
        XCTAssertFalse(c.workspace.document!.page(at: 0)!.annotations.contains { $0.type == "Link" })
    }

    func testRealRedactionRemovesTextInsteadOfAddingOverlayAnnotation() async throws {
        let c = try controller(); let input = try c.workspace.engineInput(); let page = try inspect(input)
        let span = try XCTUnwrap(page.spans.first { $0.text == "Original target" })
        var request = ContentEditRequest(operation: "redact", page: 0); request.bbox = span.bbox
        try await c.applyContentEdit(request, input: input, engine: engine)
        XCTAssertFalse(c.workspace.document!.page(at: 0)!.string!.contains("Original target"))
        XCTAssertTrue(c.workspace.document!.page(at: 0)!.string!.contains("Neighbor preserved"))
        XCTAssertFalse(c.workspace.document!.page(at: 0)!.annotations.contains { $0.type == "Redact" || $0.type == "Square" })
        c.undo(); XCTAssertTrue(c.workspace.document!.page(at: 0)!.string!.contains("Original target"))
    }

    func testTooLongReplacementFailsWithoutDocumentOrHistoryChange() async throws {
        let c = try controller(); let input = try c.workspace.engineInput(); let page = try inspect(input)
        var request = ContentEditRequest(operation: "replace-text", page: 0)
        request.id = try XCTUnwrap(page.spans.first { $0.text == "Original target" }).id; request.text = String(repeating: "TOO LONG ", count: 30)
        do { try await c.applyContentEdit(request, input: input, engine: engine); XCTFail("Must refuse overflow") } catch {}
        XCTAssertFalse(c.workspace.canUndo); XCTAssertFalse(c.workspace.isDirty)
        XCTAssertTrue(c.workspace.document!.page(at: 0)!.string!.contains("Original target"))
    }

    func testStaleInspectionTokenNeverModifiesReplacementDocument() async throws {
        let c = try controller(); let input = try c.workspace.engineInput(); let page = try inspect(input)
        var request = ContentEditRequest(operation: "replace-text", page: 0)
        request.id = try XCTUnwrap(page.spans.first { $0.text == "Original target" }).id; request.text = "Edited target"
        try c.workspace.createBlank(); c.refreshDocument()
        do { try await c.applyContentEdit(request, input: input, engine: engine); XCTFail("Must reject stale inspection") }
        catch { XCTAssertEqual(error as? PDFWorkspaceError, .staleEngineResult) }
        XCTAssertFalse(c.workspace.isDirty); XCTAssertFalse(c.workspace.canUndo)
    }

    func testImportedRawActiveContentCannotBeWashedAwayByPDFKit() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-admission-test-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: dir) }
        let unsafe = dir.appendingPathComponent("open-action.pdf")
        let objects = ["<< /Type /Catalog /Pages 2 0 R /OpenAction [3 0 R /Fit] >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 420 594] /Resources << >> /Contents 4 0 R >>", "<< /Length 0 >>\nstream\n\nendstream"]
        var pdf = "%PDF-1.7\n", offsets = [0]
        for (index, object) in objects.enumerated() { offsets.append(pdf.utf8.count); pdf += "\(index + 1) 0 obj\n\(object)\nendobj\n" }
        let xref = pdf.utf8.count
        pdf += "xref\n0 5\n0000000000 65535 f \n" + offsets.dropFirst().map { String(format: "%010d 00000 n \n", $0) }.joined()
        pdf += "trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n\(xref)\n%%EOF\n"
        try Data(pdf.utf8).write(to: unsafe)
        let c = try controller(); try c.workspace.insertPDF(unsafe, at: 1)
        let input = try c.workspace.engineInput()
        XCTAssertTrue(input.safetySourceDatas.contains(try Data(contentsOf: unsafe)))
        let page = try inspect(input)
        var request = ContentEditRequest(operation: "replace-text", page: 0)
        request.id = try XCTUnwrap(page.spans.first { $0.text == "Original target" }).id; request.text = "Edited target"
        let revision = c.workspace.revision
        do { try await c.applyContentEdit(request, input: input, engine: engine); XCTFail("Raw active source must refuse") } catch {}
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertTrue(c.workspace.document!.page(at: 0)!.string!.contains("Original target"))
    }
}
