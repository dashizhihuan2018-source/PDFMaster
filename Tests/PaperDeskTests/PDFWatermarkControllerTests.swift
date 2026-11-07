import XCTest
import AppKit
import PDFKit
@testable import PaperCore
@testable import PaperDesk

@MainActor final class PDFWatermarkControllerTests: XCTestCase {
    private func controller(watermarks: Bool = true) throws -> (WorkbenchController, URL, Data) {
        _ = NSApplication.shared
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("OrdinaryWatermark-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let source = directory.appendingPathComponent("original.pdf"), document = PDFDocument()
        for index in 0..<2 {
            let page = PDFPage(); page.setBounds(CGRect(x: 0, y: 0, width: 400, height: 200), for: .mediaBox)
            if watermarks {
                let mark = PDFAnnotation(bounds: CGRect(x: 20, y: 20, width: 100, height: 30), forType: PDFAnnotationSubtype(rawValue: "Watermark"), withProperties: nil)
                mark.contents = "Explicit watermark \(index)"; page.addAnnotation(mark)
            }
            document.insert(page, at: index)
        }
        try XCTUnwrap(document.dataRepresentation()).write(to: source)
        let c = WorkbenchController(); _ = c.view; try c.workspace.open(source); c.refreshDocument()
        return (c, source, try Data(contentsOf: source))
    }
    private func dependencies(confirm: Bool = true) -> PDFWatermarkDependencies {
        PDFWatermarkDependencies(confirm: { _, _ in confirm }, notify: { _, _ in }, progress: { _, _, _ in {} })
    }
    func testWatermarkDoesNotAddACardOrKeepTheRejectedPreviewPanel() {
        XCTAssertNil(ToolCatalog.descriptor(id: "remove-watermark")); XCTAssertEqual(ToolCatalog.all.count, 28)
        XCTAssertNil(NSClassFromString("PaperDesk.PDFWatermarkPanel"))
    }
    func testNoCandidatesOnlyNotifiesAndNeverConfirmsOrRemoves() async throws {
        let (c, source, original) = try controller(watermarks: false)
        var confirmations = 0, messages: [String] = [], phases: [PDFWatermarkPhase] = []
        let services = PDFWatermarkDependencies(confirm: { _, _ in confirmations += 1; return true }, notify: { _, text in messages.append(text) }, progress: { _, _, _ in {} }, runner: { phase, data, flag, operation in
            phases.append(phase); return try await LocalToolJob.run(data: data, cancellation: flag, operation: operation)
        })
        let result = try await c.performWatermarkRemoval(dependencies: services)
        XCTAssertEqual(result, .noCandidates); XCTAssertEqual(confirmations, 0); XCTAssertEqual(messages.count, 1)
        XCTAssertTrue(messages[0].hasPrefix("未检测到可直接删除的水印。")); XCTAssertTrue(messages[0].contains("扫描")); XCTAssertFalse(phases.contains(.removal))
        XCTAssertFalse(c.workspace.isDirty); XCTAssertFalse(c.workspace.canUndo); XCTAssertEqual(try Data(contentsOf: source), original)
    }
    func testConfirmationCancellationDoesNotRunRemovalOrChangeUnsavedDocument() async throws {
        let (c, source, original) = try controller()
        try c.workspace.addText("Unsaved content", page: 0, bounds: CGRect(x: 130, y: 70, width: 220, height: 30))
        let revision = c.workspace.revision, document = c.workspace.document
        var phases: [PDFWatermarkPhase] = [], confirmations = 0
        let services = PDFWatermarkDependencies(confirm: { _, count in confirmations += 1; XCTAssertEqual(count, 2); return false }, notify: { _, _ in XCTFail("Candidates exist") }, progress: { _, _, _ in {} }, runner: { phase, data, flag, operation in
            phases.append(phase); return try await LocalToolJob.run(data: data, cancellation: flag, operation: operation)
        })
        let result = try await c.performWatermarkRemoval(dependencies: services)
        XCTAssertEqual(result, .cancelled); XCTAssertEqual(confirmations, 1); XCTAssertFalse(phases.contains(.removal))
        XCTAssertTrue(c.workspace.document === document); XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertTrue(c.workspace.isDirty); XCTAssertEqual(try Data(contentsOf: source), original)
    }
    func testConfirmationCommitsOnceInCurrentDocumentWithUndoRedoAndNoDiskWrite() async throws {
        let (c, source, original) = try controller()
        try c.workspace.addText("Unsaved content", page: 0, bounds: CGRect(x: 130, y: 70, width: 220, height: 30))
        c.refreshDocument(preservePage: 1); c.pdfView.scaleFactor = 1.37
        let revision = c.workspace.revision
        var confirmations = 0
        let services = PDFWatermarkDependencies(confirm: { _, count in confirmations += 1; XCTAssertEqual(count, 2); return true }, notify: { _, _ in XCTFail("Candidates exist") }, progress: { _, _, _ in {} })
        XCTAssertEqual(services.progressDelayNanoseconds, 500_000_000)
        let result = try await c.performWatermarkRemoval(dependencies: services)
        XCTAssertEqual(result, .removed(2)); XCTAssertEqual(confirmations, 1)
        XCTAssertEqual(c.workspace.revision, revision + 1); XCTAssertTrue(c.workspace.isDirty)
        XCTAssertEqual(c.workspace.sourceURL, source); XCTAssertEqual(c.pdfView.document?.index(for: try XCTUnwrap(c.pdfView.currentPage)), 1); XCTAssertEqual(c.pdfView.scaleFactor, 1.37, accuracy: 0.001)
        XCTAssertFalse(c.workspace.document!.page(at: 0)!.annotations.contains { $0.type == "Watermark" })
        XCTAssertFalse(c.workspace.document!.page(at: 1)!.annotations.contains { $0.type == "Watermark" })
        XCTAssertTrue(c.workspace.document!.page(at: 0)!.annotations.contains { $0.contents == "Unsaved content" })
        c.undo(); XCTAssertTrue(c.workspace.document!.page(at: 0)!.annotations.contains { $0.type == "Watermark" })
        XCTAssertTrue(c.workspace.document!.page(at: 0)!.annotations.contains { $0.contents == "Unsaved content" })
        c.redo(); XCTAssertFalse(c.workspace.document!.page(at: 0)!.annotations.contains { $0.type == "Watermark" })
        let saved = source.deletingLastPathComponent().appendingPathComponent("saved.pdf")
        try c.workspace.saveCopy(to: saved)
        let reopened = try XCTUnwrap(PDFDocument(url: saved))
        XCTAssertFalse(reopened.page(at: 0)!.annotations.contains { $0.type == "Watermark" })
        XCTAssertFalse(reopened.page(at: 1)!.annotations.contains { $0.type == "Watermark" })
        XCTAssertTrue(reopened.page(at: 0)!.annotations.contains { $0.contents == "Unsaved content" })
        XCTAssertEqual(try Data(contentsOf: source), original); XCTAssertFalse(c.isLocalToolRunning); XCTAssertFalse(LocalToolActivity.hasRunningJobs)
    }
    func testStaleReplacementDuringRemovalNeverReceivesResult() async throws {
        let (c, source, original) = try controller()
        var services = dependencies()
        services.runner = { phase, data, flag, operation in
            if phase == .removal { try c.workspace.createBlank(); c.refreshDocument() }
            return try await LocalToolJob.run(data: data, cancellation: flag, operation: operation)
        }
        do { _ = try await c.performWatermarkRemoval(dependencies: services); XCTFail("Stale result must reject") }
        catch { XCTAssertEqual(error as? PDFWorkspaceError, .staleEngineResult) }
        XCTAssertEqual(c.workspace.document?.pageCount, 1); XCTAssertFalse(c.workspace.canUndo); XCTAssertFalse(c.workspace.isDirty)
        XCTAssertEqual(try Data(contentsOf: source), original)
    }
    func testRemovalFailureAndCancellationPreserveDocumentHistoryAndSource() async throws {
        for cancel in [false, true] {
            let (c, source, original) = try controller()
            let document = c.workspace.document, revision = c.workspace.revision
            var services = dependencies()
            services.runner = { phase, data, flag, operation in
                if phase == .removal {
                    if cancel { flag.cancel(); throw OrganizeError.cancelled }
                    throw OrganizeError.processFailed("Synthetic failure")
                }
                return try await LocalToolJob.run(data: data, cancellation: flag, operation: operation)
            }
            do { _ = try await c.performWatermarkRemoval(dependencies: services); XCTFail("Must reject") } catch {}
            XCTAssertTrue(c.workspace.document === document); XCTAssertEqual(c.workspace.revision, revision)
            XCTAssertFalse(c.workspace.canUndo); XCTAssertFalse(c.workspace.isDirty)
            XCTAssertEqual(try Data(contentsOf: source), original); XCTAssertFalse(c.isLocalToolRunning)
        }
    }
    func testSlowScanShowsCancelableProgressOnlyAfterDelayAndCleansUp() async throws {
        let (c, _, _) = try controller()
        var shows = 0, dismissals = 0, confirmations = 0
        let services = PDFWatermarkDependencies(confirm: { _, _ in confirmations += 1; return true }, notify: { _, _ in }, progress: { _, _, cancel in
            shows += 1; cancel(); return { dismissals += 1 }
        }, runner: { phase, data, flag, operation in
            if phase == .scan { try await Task.sleep(nanoseconds: 650_000_000) }
            return try await LocalToolJob.run(data: data, cancellation: flag, operation: operation)
        })
        do { _ = try await c.performWatermarkRemoval(dependencies: services); XCTFail("User canceled slow scan") }
        catch { XCTAssertEqual(error as? OrganizeError, .cancelled) }
        XCTAssertEqual(shows, 1); XCTAssertEqual(dismissals, 1); XCTAssertEqual(confirmations, 0)
        XCTAssertFalse(c.workspace.canUndo); XCTAssertFalse(c.isLocalToolRunning); XCTAssertFalse(LocalToolActivity.hasRunningJobs)
    }
    func testRawSignatureRejectionPrecedesSerializationAndNeverRunsScan() async throws {
        var serialized = false
        do {
            let _: Data = try await PDFWatermarkAdmissionBoundary.prepare(sources: [Data("synthetic-signed".utf8)], validate: { _ in throw OrganizeError.signed }, serialize: { serialized = true; return Data() })
            XCTFail("Signed source must reject")
        } catch { XCTAssertFalse(serialized) }
        let (c, source, bytes) = try controller()
        var phases: [PDFWatermarkPhase] = []
        var services = dependencies()
        services.runner = { phase, _, _, _ in phases.append(phase); throw OrganizeError.signed }
        do { _ = try await c.performWatermarkRemoval(dependencies: services); XCTFail("Signed provenance must reject") }
        catch { XCTAssertEqual(error as? OrganizeError, .signed) }
        XCTAssertEqual(phases, [.admission]); XCTAssertFalse(c.workspace.canUndo); XCTAssertEqual(try Data(contentsOf: source), bytes)
    }
    func testFinishingWatermarkDoesNotClearAnotherJobsCancellationFlag() async throws {
        let (c, _, _) = try controller(watermarks: false)
        let other = LocalToolCancellation()
        var services = dependencies()
        services.runner = { phase, data, flag, operation in
            if phase == .scan { c.localToolCancellation = other }
            return try await LocalToolJob.run(data: data, cancellation: flag, operation: operation)
        }
        _ = try await c.performWatermarkRemoval(dependencies: services)
        XCTAssertTrue(c.localToolCancellation === other); c.localToolCancellation = nil
    }
    func testCancelAndWaitCleansUpInFlightCommandWithoutApplyingResult() async throws {
        let (c, source, original) = try controller()
        var started = false
        var services = dependencies()
        services.runner = { phase, data, flag, operation in
            if phase == .scan {
                started = true
                while !flag.isCancelled { try await Task.sleep(nanoseconds: 1_000_000) }
                throw OrganizeError.cancelled
            }
            return try await LocalToolJob.run(data: data, cancellation: flag, operation: operation)
        }
        let task = Task { try await c.performWatermarkRemoval(dependencies: services) }
        for _ in 0..<1000 {
            if started { break }
            try await Task.sleep(nanoseconds: 5_000_000)
        }
        guard started else {
            task.cancel()
            _ = try? await task.value
            XCTFail("Watermark scan did not start within five seconds; check engine admission/environment")
            return
        }
        XCTAssertTrue(LocalToolActivity.hasRunningJobs)
        await LocalToolActivity.cancelAndWait()
        do { _ = try await task.value; XCTFail("Cancellation must reject") }
        catch { XCTAssertEqual(error as? OrganizeError, .cancelled) }
        XCTAssertFalse(LocalToolActivity.hasRunningJobs); XCTAssertFalse(c.isLocalToolRunning)
        XCTAssertFalse(c.workspace.isDirty); XCTAssertFalse(c.workspace.canUndo)
        XCTAssertEqual(try Data(contentsOf: source), original)
    }
    func testEditingToolbarAndMenuSelectorRunSameCurrentDocumentCommand() async throws {
        let (c, source, original) = try controller()
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1100, height: 750), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = c
        defer { window.contentViewController = nil; window.close() }
        var confirmations = 0
        c.watermarkDependencies = PDFWatermarkDependencies(confirm: { _, _ in confirmations += 1; return false }, notify: { _, _ in XCTFail("Candidates exist") }, progress: { _, _, _ in {} })
        c.enterMode(.editing)
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        let button = try XCTUnwrap(descendants(c.view).compactMap { $0 as? ToolStripButton }.first { $0.toolID == "remove-watermark" })
        XCTAssertEqual(button.title, "删除水印"); XCTAssertTrue(button.isEnabled)
        button.performClick(nil)
        for _ in 0..<300 { if confirmations == 1 && !c.isLocalToolRunning { break }; try await Task.sleep(nanoseconds: 10_000_000) }
        XCTAssertEqual(confirmations, 1); XCTAssertFalse(c.isLocalToolRunning)
        c.removeWatermark(nil)
        for _ in 0..<300 { if confirmations == 2 && !c.isLocalToolRunning { break }; try await Task.sleep(nanoseconds: 10_000_000) }
        XCTAssertEqual(confirmations, 2); XCTAssertNil(window.attachedSheet)
        XCTAssertFalse(c.workspace.canUndo); XCTAssertFalse(c.workspace.isDirty)
        XCTAssertEqual(try Data(contentsOf: source), original)
    }
    func testFastCompletedWorkerNeverPresentsDelayedProgress() async throws {
        var shows = 0
        let services = PDFWatermarkDependencies(progress: { _, _, _ in shows += 1; return {} }, runner: { _, _, _, _ in LocalToolResult(data: Data(), message: "complete") })
        let result = try await services.run(.scan, data: Data(), cancellation: LocalToolCancellation(), window: nil, operation: { _, _, _ in "" })
        XCTAssertEqual(result.message, "complete"); XCTAssertEqual(shows, 0)
        XCTAssertEqual(services.progressDelayNanoseconds, 500_000_000)
    }
    func testNativeConfirmationUsesVerifiedOrdinaryWPSMessageAndButtons() async throws {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 600), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let task = Task { await PDFWatermarkDependencies().confirm(window, 2) }
        for _ in 0..<100 { if window.attachedSheet != nil { break }; try await Task.sleep(nanoseconds: 10_000_000) }
        let sheet = try XCTUnwrap(window.attachedSheet)
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        let views = descendants(try XCTUnwrap(sheet.contentView))
        XCTAssertTrue(views.compactMap { $0 as? NSTextField }.contains { $0.stringValue == "确定要删除已有水印吗？" })
        XCTAssertTrue(views.compactMap { $0 as? NSButton }.contains { $0.title == "确定" })
        XCTAssertTrue(views.compactMap { $0 as? NSButton }.contains { $0.title == "取消" })
        window.endSheet(sheet, returnCode: .alertSecondButtonReturn)
        let confirmed = await task.value
        XCTAssertFalse(confirmed)
    }
}
