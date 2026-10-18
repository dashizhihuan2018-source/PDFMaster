import AppKit
import PDFKit
import XCTest
import PaperCore
@testable import PaperDesk

@MainActor final class PDFCompressionPanelTests: XCTestCase {
    func testCompressionEntryPointDoesNotUseAlertSequenceOrModalLoop() throws {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let source = try String(contentsOf: root.appendingPathComponent("Sources/PaperDesk/PDFCompressionController.swift"), encoding: .utf8)
        XCTAssertFalse(source.contains("NSAlert"), "Compression must use one persistent custom panel, not sequential alerts")
        XCTAssertFalse(source.contains("runModal"))
        XCTAssertTrue(source.contains("PDFCompressionPanelController"))
    }

    func testInitialCardsHaveNoInventedSizeAndExportIsDisabled() throws {
        let input = try input()
        let session = PDFCompressionSession(input: input, worker: { _ in PDFCompressionPreview(input: input, data: input.data) })
        XCTAssertEqual(session.selectedPreset, .balanced)
        XCTAssertFalse(session.canExport)
        for preset in PDFCompressionSession.cardPresets {
            XCTAssertEqual(session.status(for: preset).label, "等待计算")
        }
        XCTAssertEqual(PDFCompressionSession.cardPresets.map(\.imageDPI), [300, 150, 72])
    }

    func testAllCardsCalculateSeriallyAndSelectionUsesItsOwnRealBytes() async throws {
        let input = try input(), gate = CompressionWorkerGate()
        let session = PDFCompressionSession(input: input, worker: gate.run)
        session.start()
        await gate.waitForCallCount(1)
        XCTAssertEqual(gate.calls, [.balanced])
        session.select(.smallSize)
        XCTAssertFalse(session.canExport)
        XCTAssertEqual(gate.calls.count, 1)
        gate.succeed(input, count: 200)
        await gate.waitForCallCount(2)
        XCTAssertEqual(gate.calls[1], .smallSize)
        gate.succeed(input, count: 100)
        await gate.waitForCallCount(3)
        XCTAssertEqual(session.selectedPreview?.data.count, 100)
        XCTAssertTrue(session.canExport)
        XCTAssertTrue(session.status(for: .balanced).label.contains("%"))
        XCTAssertEqual(gate.maximumActive, 1)
        session.close(); gate.succeed(input, count: 300)
        await Task.yield()
        XCTAssertFalse(session.canExport)
    }

    func testClosedSessionIgnoresLateSuccessAndCancelsItsWorkerOnce() async throws {
        let input = try input(), gate = CompressionWorkerGate()
        var cancellations = 0
        let session = PDFCompressionSession(input: input, worker: gate.run, cancelWorker: { cancellations += 1 })
        session.start(); await gate.waitForCallCount(1)
        session.close(); session.close()
        gate.succeed(input, count: 100)
        for _ in 0..<20 { await Task.yield() }
        XCTAssertEqual(cancellations, 1)
        XCTAssertTrue(session.isClosed)
        XCTAssertNil(session.selectedPreview)
        XCTAssertEqual(gate.calls.count, 1)
    }

    func testStaleSnapshotDisablesExportAndRejectsLateResults() async throws {
        let input = try input(), gate = CompressionWorkerGate()
        var fresh = true
        let session = PDFCompressionSession(input: input, worker: gate.run, validate: { if !fresh { throw PDFWorkspaceError.staleEngineResult } })
        session.start(); await gate.waitForCallCount(1)
        fresh = false; session.checkFreshness()
        gate.succeed(input, count: 100)
        for _ in 0..<20 { await Task.yield() }
        XCTAssertTrue(session.isStale)
        XCTAssertFalse(session.canExport)
        XCTAssertNil(session.selectedPreview)
        XCTAssertTrue(session.message.contains("已变化"))
    }

    func testNoSavingsIsReportedHonestlyAndCannotExport() async throws {
        let input = try input()
        let session = PDFCompressionSession(input: input, worker: { _ in PDFCompressionPreview(input: input, data: input.data) })
        session.start()
        for _ in 0..<100 { await Task.yield() }
        XCTAssertFalse(session.canExport)
        XCTAssertTrue(session.status(for: .balanced).label.contains("无进一步减小"))
    }

    func testFailureIsVisibleAndRetryRecalculatesWithoutLosingOtherResults() async throws {
        let input = try input(), gate = CompressionWorkerGate()
        let session = PDFCompressionSession(input: input, worker: gate.run)
        session.start(); await gate.waitForCallCount(1)
        gate.fail(); await gate.waitForCallCount(2)
        XCTAssertTrue(session.status(for: .balanced).isFailure)
        session.retrySelected()
        gate.succeed(input, count: 200); await gate.waitForCallCount(3)
        XCTAssertEqual(gate.calls[2], .balanced)
        gate.succeed(input, count: 100); await gate.waitForCallCount(4)
        XCTAssertTrue(session.canExport)
        XCTAssertEqual(session.selectedPreview?.data.count, 100)
        session.close(); gate.succeed(input, count: 150)
    }

    func testPanelsAreIndependentAndLosslessIsAnAdditionalChoice() async throws {
        let input = try input(), firstGate = CompressionWorkerGate(), secondGate = CompressionWorkerGate()
        let first = PDFCompressionSession(input: input, worker: firstGate.run)
        let second = PDFCompressionSession(input: input, worker: secondGate.run)
        first.start(); second.start()
        await firstGate.waitForCallCount(1); await secondGate.waitForCallCount(1)
        first.select(.lossless)
        firstGate.succeed(input, count: 200); await firstGate.waitForCallCount(2)
        XCTAssertEqual(firstGate.calls[1], .lossless)
        XCTAssertEqual(second.selectedPreset, .balanced)
        first.close(); second.close()
        firstGate.succeed(input, count: 100); secondGate.succeed(input, count: 100)
    }

    func testCustomPanelContainsThreeCardsAndActualPDFKitDocumentWithoutBlockingParent() throws {
        _ = NSApplication.shared
        let workbench = WorkbenchController(); _ = workbench.view
        try workbench.workspace.createBlank()
        let parent = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1200, height: 800), styleMask: [.titled], backing: .buffered, defer: false)
        let panel = try PDFCompressionPanelController(workbench: workbench, parent: parent, qpdf: URL(fileURLWithPath: "/unused-qpdf"))
        defer { panel.close() }
        XCTAssertTrue(panel.window is NSPanel)
        XCTAssertNil(parent.attachedSheet)
        XCTAssertNil(panel.window?.sheetParent)
        let views = descendants(try XCTUnwrap(panel.window?.contentView))
        let cardIDs = views.compactMap { $0.identifier?.rawValue }.filter { $0.hasPrefix("ReduceFileSizeQualityButton-") }
        XCTAssertEqual(Set(cardIDs), Set(["ReduceFileSizeQualityButton-300", "ReduceFileSizeQualityButton-150", "ReduceFileSizeQualityButton-72"]))
        let preview = try XCTUnwrap(views.first { $0.identifier?.rawValue == "CompressionPDFPreview" } as? PDFView)
        XCTAssertEqual(preview.document?.pageCount, 1)
        let save = try XCTUnwrap(views.first { $0.identifier?.rawValue == "CompressionSaveAs" } as? NSButton)
        XCTAssertFalse(save.isEnabled)
        panel.window?.contentView?.layoutSubtreeIfNeeded()
        for card in views where card.identifier?.rawValue.hasPrefix("ReduceFileSizeQualityButton-") == true {
            XCTAssertEqual(card.frame.height, 96, accuracy: 0.5)
            XCTAssertEqual(card.frame.width, 330, accuracy: 0.5)
        }
        panel.close()
        XCTAssertNil(preview.document)
    }

    func testDocumentMutationMarksActualPanelStaleWithoutReplacingOriginalSnapshot() async throws {
        _ = NSApplication.shared
        let workbench = WorkbenchController(); _ = workbench.view
        try workbench.workspace.createBlank()
        let parent = NSWindow(contentRect: .zero, styleMask: [.titled], backing: .buffered, defer: false)
        let panel = try PDFCompressionPanelController(workbench: workbench, parent: parent, qpdf: URL(fileURLWithPath: "/unused-qpdf"))
        defer { panel.close() }
        let initial = panel.session.input.data
        try workbench.workspace.createBlank()
        for _ in 0..<50 { await Task.yield() }
        XCTAssertTrue(panel.session.isStale)
        XCTAssertEqual(panel.session.input.data, initial)
        XCTAssertFalse(panel.session.canExport)
    }

    func testClosingActualPanelOnlyRunsReleaseCallbackOnce() throws {
        _ = NSApplication.shared
        let workbench = WorkbenchController(); _ = workbench.view; try workbench.workspace.createBlank()
        let parent = NSWindow(contentRect: .zero, styleMask: [.titled], backing: .buffered, defer: false)
        let panel = try PDFCompressionPanelController(workbench: workbench, parent: parent, qpdf: URL(fileURLWithPath: "/unused-qpdf"))
        var closes = 0
        panel.onClose = { closes += 1 }
        panel.close(); panel.close()
        XCTAssertEqual(closes, 1)
        XCTAssertTrue(panel.session.isClosed)
    }

    func testSaveFailureRemainsVisibleAcrossBackgroundResultsAndCanRetry() async throws {
        let input = try input(), gate = CompressionWorkerGate()
        let session = PDFCompressionSession(input: input, worker: gate.run)
        session.start(); await gate.waitForCallCount(1)
        gate.succeed(input, count: 100); await gate.waitForCallCount(2)
        let target = URL(fileURLWithPath: "/unused-compressed.pdf")
        XCTAssertFalse(session.saveCopy(to: target) { _, _ in throw OrganizeError.outputExists })
        XCTAssertTrue(session.exportFailed)
        XCTAssertTrue(session.exportFeedback?.contains("新的文件名") == true)
        let error = session.exportFeedback
        gate.succeed(input, count: 200); await gate.waitForCallCount(3)
        XCTAssertEqual(session.exportFeedback, error)
        var writtenCount = 0
        XCTAssertTrue(session.saveCopy(to: target) { preview, destination in
            XCTAssertEqual(destination, target); XCTAssertEqual(preview.after, 100); writtenCount += 1
        })
        XCTAssertFalse(session.exportFailed)
        XCTAssertEqual(writtenCount, 1)
        session.close(); gate.succeed(input, count: 150)
        XCTAssertFalse(session.saveCopy(to: target) { _, _ in XCTFail("Closed sessions must never write") })
    }

    private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }

    private func input() throws -> PDFEngineInput {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        return try workspace.engineInput()
    }
}

@MainActor private final class CompressionWorkerGate {
    var calls: [PDFCompressionPreset] = []
    var active = 0
    var maximumActive = 0
    var continuation: CheckedContinuation<PDFCompressionPreview, Error>?
    func run(_ preset: PDFCompressionPreset) async throws -> PDFCompressionPreview {
        calls.append(preset); active += 1; maximumActive = max(maximumActive, active)
        defer { active -= 1 }
        return try await withCheckedThrowingContinuation { continuation = $0 }
    }
    func succeed(_ input: PDFEngineInput, count: Int) {
        let next = continuation; continuation = nil
        next?.resume(returning: PDFCompressionPreview(input: input, data: Data(repeating: 0, count: count)))
    }
    func fail() { let next = continuation; continuation = nil; next?.resume(throwing: OrganizeError.unavailable) }
    func waitForCallCount(_ count: Int) async {
        for _ in 0..<2000 {
            if calls.count >= count { return }
            await Task.yield()
        }
        XCTFail("Worker did not receive expected call \(count); received \(calls.count)")
    }
}
