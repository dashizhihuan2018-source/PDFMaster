import XCTest
import AppKit
import PaperCore
@testable import PaperDesk

@MainActor final class OrganizeIntegrationTests: XCTestCase {
    private func requireEngine() throws -> URL {
        guard let engine = ExternalToolDiscovery.qpdf() else { throw XCTSkip("Optional qpdf not found; set PDFMASTER_QPDF or PATH") }
        return engine
    }

    func testQuitCancelsAndWaitsForPrivateJobCleanup() async throws {
        let engine = try requireEngine()
        let c = WorkbenchController(); _ = c.view; try c.workspace.createBlank(); c.refreshDocument()
        let task = Task { try await c.applyLocalTool(.compress, qpdf: engine) }
        while !c.isLocalToolRunning { await Task.yield() }
        XCTAssertTrue(LocalToolActivity.hasRunningJobs)
        await LocalToolActivity.cancelAndWait()
        XCTAssertFalse(LocalToolActivity.hasRunningJobs)
        XCTAssertFalse(c.isLocalToolRunning)
        do { try await task.value; XCTFail("must cancel") } catch { XCTAssertEqual(error as? OrganizeError, .cancelled) }
        XCTAssertFalse(c.workspace.isDirty)
    }

    func testChangedDocumentRejectsInFlightResult() async throws {
        let engine = try requireEngine()
        let c = WorkbenchController(); _ = c.view; try c.workspace.createBlank(); c.refreshDocument()
        let task = Task { try await c.applyLocalTool(.compress, qpdf: engine) }
        while !c.isLocalToolRunning { await Task.yield() }
        try c.workspace.createBlank()
        do { try await task.value; XCTFail("must reject obsolete result") } catch { XCTAssertEqual(error as? PDFWorkspaceError, .staleEngineResult) }
        XCTAssertFalse(c.workspace.isDirty)
    }

    func testBackgroundNumbersUpdatePresentationAndUndo() async throws {
        let c = WorkbenchController(); _ = c.view; try c.workspace.createBlank(); c.refreshDocument()
        let engine = try requireEngine()
        var options = NumberOptions(); options.prefix = "TEST-"; options.start = 17
        try await c.applyLocalTool(.numbers(options), qpdf: engine)
        XCTAssertFalse(c.isLocalToolRunning)
        XCTAssertTrue(c.pdfView.document?.string?.contains("TEST-17") == true)
        XCTAssertTrue(c.workspace.canUndo)
        c.workspace.undo(); c.refreshDocument()
        XCTAssertFalse(c.pdfView.document?.string?.contains("TEST-17") == true)
        XCTAssertFalse(c.workspace.isDirty)
    }

    func testIntegratedToolsAreExplicitlyPartial() {
        for id in ["page-numbers", "ocr", "compress"] {
            XCTAssertEqual(ToolCatalog.descriptor(id: id)?.availability, .partial)
        }
    }
}
