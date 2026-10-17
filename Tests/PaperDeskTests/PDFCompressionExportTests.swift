import AppKit
import PDFKit
import XCTest
import PaperCore
@testable import PaperDesk

@MainActor final class PDFCompressionExportTests: XCTestCase {
    func testFileMenuCompressionActionIsAvailableOnWorkbench() {
        let controller = WorkbenchController()
        XCTAssertTrue(controller.responds(to: NSSelectorFromString("reduceFileSize:")))
    }

    private var qpdf: URL {
        get throws {
            guard let engine = ExternalToolDiscovery.qpdf() else { throw XCTSkip("Optional qpdf not found; set PDFMASTER_QPDF or PATH") }
            return engine
        }
    }

    func testPrepareAndExportPreserveDirtyEditsAndExactCompressedBytes() async throws {
        let folder = try directory(); defer { try? FileManager.default.removeItem(at: folder) }
        let source = folder.appendingPathComponent("source.pdf"), target = folder.appendingPathComponent("compressed.pdf")
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); try c.workspace.save(to: source)
        let original = try Data(contentsOf: source)
        try c.workspace.addPopupNote("Keep unsaved note", author: nil, page: 0, bounds: CGRect(x: 20, y: 20, width: 30, height: 30), color: .yellow)
        let revision = c.workspace.revision
        let preview = try await c.prepareCompression(preset: .balanced, qpdf: qpdf)
        XCTAssertTrue(preview.reduced)
        XCTAssertTrue(preview.summary.contains("%"))
        try c.exportCompression(preview, to: target)
        XCTAssertEqual(try Data(contentsOf: target), preview.data)
        XCTAssertEqual(try Data(contentsOf: source), original)
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertTrue(c.workspace.isDirty)
        XCTAssertTrue(c.workspace.canUndo)
        XCTAssertEqual(c.workspace.sourceURL, source)
        XCTAssertEqual(PDFDocument(url: target)?.page(at: 0)?.annotations.first?.contents, "Keep unsaved note")
        c.workspace.undo()
        XCTAssertTrue(c.workspace.document?.page(at: 0)?.annotations.isEmpty == true)
    }

    func testExportRefusesExistingFileSourceAliasAndStalePreview() async throws {
        let folder = try directory(); defer { try? FileManager.default.removeItem(at: folder) }
        let source = folder.appendingPathComponent("source.pdf"), target = folder.appendingPathComponent("existing.pdf")
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); try c.workspace.save(to: source)
        let preview = try await c.prepareCompression(preset: .lossless, qpdf: qpdf)
        XCTAssertTrue(preview.reduced)
        let original = try Data(contentsOf: source)
        let sentinel = Data("Do not replace".utf8); try sentinel.write(to: target)
        XCTAssertThrowsError(try c.exportCompression(preview, to: target)) { XCTAssertEqual($0 as? OrganizeError, .outputExists) }
        XCTAssertThrowsError(try c.exportCompression(preview, to: source)) { XCTAssertEqual($0 as? OrganizeError, .outputExists) }
        let alias = folder.appendingPathComponent("alias.pdf")
        try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: source)
        XCTAssertThrowsError(try c.exportCompression(preview, to: alias))
        XCTAssertEqual(try Data(contentsOf: source), original)
        XCTAssertEqual(try Data(contentsOf: target), sentinel)
        try c.workspace.addPopupNote("Newer edit", author: nil, page: 0, bounds: CGRect(x: 20, y: 20, width: 30, height: 30), color: .yellow)
        let unused = folder.appendingPathComponent("unused.pdf")
        XCTAssertThrowsError(try c.exportCompression(preview, to: unused)) { XCTAssertEqual($0 as? PDFWorkspaceError, .staleEngineResult) }
        XCTAssertFalse(FileManager.default.fileExists(atPath: unused.path))
    }

    func testNoSavingsPreviewDoesNotCreateFileOrMutateDocument() throws {
        let folder = try directory(); defer { try? FileManager.default.removeItem(at: folder) }
        let c = WorkbenchController(); _ = c.view; try c.workspace.createBlank()
        let input = try c.workspace.engineInput()
        let preview = PDFCompressionPreview(input: input, data: input.data)
        XCTAssertFalse(preview.reduced)
        XCTAssertTrue(preview.summary.contains("未能进一步缩小"))
        let target = folder.appendingPathComponent("unused.pdf")
        XCTAssertThrowsError(try c.exportCompression(preview, to: target))
        XCTAssertFalse(FileManager.default.fileExists(atPath: target.path))
        XCTAssertFalse(c.workspace.isDirty)
        XCTAssertFalse(c.workspace.canUndo)
    }

    func testCancelAndDocumentReplacementRejectPreparedResult() async throws {
        let c = WorkbenchController(); _ = c.view; try c.workspace.createBlank()
        let cancelled = Task { try await c.prepareCompression(preset: .balanced, qpdf: qpdf) }
        while !c.isLocalToolRunning { await Task.yield() }
        c.cancelLocalTool()
        do { _ = try await cancelled.value; XCTFail("Expected cancellation") }
        catch { XCTAssertEqual(error as? OrganizeError, .cancelled) }
        XCTAssertFalse(c.isLocalToolRunning)
        XCTAssertFalse(c.workspace.isDirty)
        let stale = Task { try await c.prepareCompression(preset: .balanced, qpdf: qpdf) }
        while !c.isLocalToolRunning { await Task.yield() }
        try c.workspace.createBlank()
        do { _ = try await stale.value; XCTFail("Expected stale input rejection") }
        catch { XCTAssertEqual(error as? PDFWorkspaceError, .staleEngineResult) }
        XCTAssertFalse(c.workspace.isDirty)
    }

    func testMenuRejectsNoDocumentAndRunningJob() throws {
        let c = WorkbenchController(); _ = c.view
        let item = NSMenuItem(title: "压缩文件大小…", action: NSSelectorFromString("reduceFileSize:"), keyEquivalent: "")
        XCTAssertFalse(c.validateMenuItem(item))
        try c.workspace.createBlank(); XCTAssertTrue(c.validateMenuItem(item))
        c.localToolCancellation = LocalToolCancellation(); XCTAssertFalse(c.validateMenuItem(item))
        c.localToolCancellation = nil
    }

    private func directory() throws -> URL {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("PDFCompressionExport-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        return folder
    }
}
