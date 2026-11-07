import AppKit
import PaperCore
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor final class PDFWatermarkShutdownTests: XCTestCase {
    private func shell() -> ApplicationShellController {
        _ = NSApplication.shared
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("WatermarkShutdown-\(UUID())")
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        return ApplicationShellController(recentStore: RecentDocumentStore(url: root.appendingPathComponent("recent.json")))
    }
    private func slowCommand(_ controller: WorkbenchController) -> Task<PDFWatermarkOutcome, Error> {
        let dependencies = PDFWatermarkDependencies(confirm: { _, _ in XCTFail("Canceled before confirmation"); return false }, notify: { _, _ in }, progress: { _, _, _ in {} }, runner: { _, _, flag, _ in
            while !flag.isCancelled { try await Task.sleep(nanoseconds: 1_000_000) }
            throw OrganizeError.cancelled
        })
        return Task { try await controller.performWatermarkRemoval(dependencies: dependencies) }
    }
    private struct WaitTimeout: Error, CustomStringConvertible {
        let location: String
        var description: String { "Timed out waiting for required shutdown state at \(location)" }
    }
    private func waitUntil(attempts: Int = 1000, file: StaticString = #filePath, line: UInt = #line, _ predicate: () -> Bool) async throws {
        for _ in 0..<attempts { if predicate() { return }; try await Task.sleep(nanoseconds: 5_000_000) }
        // An uncaught timeout fails the XCTest at its call site and prevents
        // dependent awaits/assertions from proceeding with an invalid state.
        throw WaitTimeout(location: "\(file):\(line)")
    }
    func testWaitUntilTimeoutThrowsBeforeDependentWork() async throws {
        var continued = false
        do {
            try await waitUntil(attempts: 2) { false }
            continued = true
            XCTFail("A timed-out state wait must throw, not continue the test")
        } catch {
            XCTAssertTrue(String(describing: error).contains("Timed out"))
        }
        XCTAssertFalse(continued)
        var checks = 0
        try await waitUntil { checks += 1; return checks == 2 }
        XCTAssertEqual(checks, 2)
    }
    func testCloseOtherTabsRejectsRunningWatermarkWithoutChangingTaskDocumentOrForeignSheet() async throws {
        let shell = shell(), kept = shell.createBlankDocument(), other = shell.createBlankDocument()
        shell.activate(tabID: kept.id)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentViewController = shell
        defer { window.contentViewController = nil; window.close() }
        let document = other.controller.workspace.document, revision = other.controller.workspace.revision
        let task = slowCommand(other.controller)
        try await waitUntil { other.controller.watermarkJob != nil }
        let job = try XCTUnwrap(other.controller.watermarkJob)
        let foreign = NSAlert(); foreign.messageText = "Unrelated sheet"; foreign.addButton(withTitle: "Keep")
        foreign.beginSheetModal(for: window) { _ in }
        XCTAssertFalse(kept.controller.readerCommands.run(.closeOtherTabs))
        XCTAssertEqual(shell.documentTabs.map(\.id), [kept.id, other.id]); XCTAssertEqual(shell.activeTabID, kept.id)
        XCTAssertTrue(other.controller.workspace.document === document); XCTAssertEqual(other.controller.workspace.revision, revision)
        XCTAssertFalse(other.controller.workspace.isDirty); XCTAssertFalse(other.controller.workspace.canUndo)
        XCTAssertTrue(other.controller.watermarkJob === job); XCTAssertFalse(job.cancellation.isCancelled)
        XCTAssertTrue(window.attachedSheet === foreign.window)
        XCTAssertTrue(kept.controller.interactionStatusText.contains("水印")); XCTAssertTrue(kept.controller.interactionStatusText.contains("关闭其他标签"))
        job.cancel(); _ = try? await task.value
        window.endSheet(foreign.window)
        try await waitUntil { window.attachedSheet == nil }
        XCTAssertTrue(kept.controller.readerCommands.run(.closeOtherTabs), "Without a job, retain the existing close-others route")
        XCTAssertEqual(shell.documentTabs.map(\.id), [kept.id]); XCTAssertFalse(LocalToolActivity.hasRunningJobs)
    }
    func testCloseTabCancelsItsWatermarkThenAppliesNormalDirtyDecision() async throws {
        let shell = shell(), tab = shell.createBlankDocument()
        try tab.controller.workspace.addText("Unsaved draft", page: 0, bounds: CGRect(x: 20, y: 40, width: 220, height: 30))
        let task = slowCommand(tab.controller)
        try await waitUntil { tab.controller.isLocalToolRunning }
        var decisions = 0
        XCTAssertFalse(shell.close(tabID: tab.id, dirtyDecision: { decisions += 1; return true }), "Owned worker requires async cancellation before closing")
        // Worker cleanup precedes the separately scheduled close continuation.
        // Await the requested close, not the intermediate worker-idle state.
        // A broken close still fails via the bounded wait, rather than racing
        // its dirty-document decision on a busy MainActor.
        try await waitUntil { decisions == 1 && !shell.documentTabs.contains { $0.id == tab.id } }
        XCTAssertFalse(tab.controller.isLocalToolRunning)
        XCTAssertFalse(shell.documentTabs.contains { $0.id == tab.id })
        XCTAssertEqual(decisions, 1)
        // Also clean up the intentional pre-fix RED run, without hanging XCTest.
        tab.controller.cancelLocalTool()
        _ = try? await task.value
        XCTAssertFalse(LocalToolActivity.hasRunningJobs)
    }
    func testWindowCloseCancelsOnlyWatermarksInThatWindow() async throws {
        let first = shell(), delegate = AppDelegate(controller: first), oldMenu = NSApp.mainMenu
        delegate.applicationDidFinishLaunching(Notification(name: NSApplication.didFinishLaunchingNotification))
        let second = delegate.newWindow(makeKey: false)
        defer {
            for session in delegate.windowSessions { session.window.delegate = nil; session.window.contentViewController = nil; session.window.close() }
            NSApp.mainMenu = oldMenu
        }
        let firstTab = first.createBlankDocument(), secondTab = second.controller.createBlankDocument()
        let firstTask = slowCommand(firstTab.controller), secondTask = slowCommand(secondTab.controller)
        try await waitUntil { firstTab.controller.isLocalToolRunning && secondTab.controller.isLocalToolRunning }
        XCTAssertFalse(delegate.windowShouldClose(second.window))
        XCTAssertFalse(delegate.windowShouldClose(second.window), "Repeated close must not enqueue a second request")
        try await waitUntil { !delegate.windowSessions.contains { $0.window === second.window } }
        XCTAssertFalse(secondTab.controller.isLocalToolRunning)
        XCTAssertTrue(firstTab.controller.isLocalToolRunning)
        XCTAssertFalse(delegate.windowSessions.contains { $0.window === second.window })
        firstTab.controller.cancelLocalTool(); secondTab.controller.cancelLocalTool()
        _ = try? await firstTask.value; _ = try? await secondTask.value
        XCTAssertFalse(LocalToolActivity.hasRunningJobs)
    }
    func testDirtyRefusalAfterCancellationKeepsTabDraftAndUndo() async throws {
        let shell = shell(), tab = shell.createBlankDocument()
        try tab.controller.workspace.addText("Keep draft", page: 0, bounds: CGRect(x: 20, y: 40, width: 220, height: 30))
        let document = tab.controller.workspace.document, revision = tab.controller.workspace.revision
        let task = slowCommand(tab.controller)
        try await waitUntil { tab.controller.isLocalToolRunning }
        var decisions = 0
        XCTAssertFalse(shell.close(tabID: tab.id, dirtyDecision: { decisions += 1; return false }))
        XCTAssertFalse(shell.close(tabID: tab.id, dirtyDecision: { XCTFail("Duplicate request"); return false }))
        try await waitUntil { decisions == 1 }
        XCTAssertEqual(decisions, 1); XCTAssertTrue(shell.documentTabs.contains { $0 === tab })
        XCTAssertTrue(tab.controller.workspace.document === document); XCTAssertEqual(tab.controller.workspace.revision, revision)
        XCTAssertTrue(tab.controller.workspace.isDirty); XCTAssertTrue(tab.controller.workspace.canUndo)
        _ = try? await task.value
        XCTAssertFalse(LocalToolActivity.hasRunningJobs)
    }
    func testQuitCancelsAllWindowWatermarksButDirtyRefusalStillPreventsQuit() async throws {
        let first = shell(), delegate = AppDelegate(controller: first), oldMenu = NSApp.mainMenu
        delegate.applicationDidFinishLaunching(Notification(name: NSApplication.didFinishLaunchingNotification))
        let second = delegate.newWindow(makeKey: false)
        defer {
            for session in delegate.windowSessions { session.window.delegate = nil; session.window.contentViewController = nil; session.window.close() }
            NSApp.mainMenu = oldMenu
        }
        let tabs = [first.createBlankDocument(), second.controller.createBlankDocument()]
        for tab in tabs { try tab.controller.workspace.addText("Keep draft", page: 0, bounds: CGRect(x: 20, y: 40, width: 200, height: 30)) }
        let documents = tabs.map { $0.controller.workspace.document }, revisions = tabs.map { $0.controller.workspace.revision }
        let tasks = tabs.map { slowCommand($0.controller) }
        try await waitUntil { tabs.allSatisfy { $0.controller.isLocalToolRunning } }
        var decisions = 0
        let approved = await delegate.prepareToTerminate(documentDecision: { _ in decisions += 1; return false })
        XCTAssertFalse(approved); XCTAssertEqual(decisions, 1); XCTAssertEqual(delegate.windowSessions.count, 2)
        for index in tabs.indices {
            _ = try? await tasks[index].value
            XCTAssertTrue(tabs[index].controller.workspace.document === documents[index])
            XCTAssertEqual(tabs[index].controller.workspace.revision, revisions[index])
            XCTAssertTrue(tabs[index].controller.workspace.isDirty); XCTAssertTrue(tabs[index].controller.workspace.canUndo)
            XCTAssertFalse(tabs[index].controller.isLocalToolRunning)
        }
        XCTAssertFalse(LocalToolActivity.hasRunningJobs)
    }
    private func openFixture(_ controller: WorkbenchController, watermark: Bool) throws -> (URL, Data) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("WatermarkOwnedSheet-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        let document = PDFDocument(), page = PDFPage()
        page.setBounds(CGRect(x: 0, y: 0, width: 400, height: 200), for: .mediaBox)
        if watermark {
            let mark = PDFAnnotation(bounds: CGRect(x: 20, y: 20, width: 100, height: 30), forType: PDFAnnotationSubtype(rawValue: "Watermark"), withProperties: nil)
            mark.contents = "Explicit watermark"; page.addAnnotation(mark)
        }
        document.insert(page, at: 0)
        let source = root.appendingPathComponent("original.pdf"), bytes = try XCTUnwrap(document.dataRepresentation())
        try bytes.write(to: source); try controller.workspace.open(source); controller.refreshDocument()
        return (source, bytes)
    }
    func testTabCloseUnblocksOwnedNativeConfirmationNoticeAndProgress() async throws {
        for stage in 0..<3 {
            let shell = shell(), tab = shell.createBlankDocument()
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1100, height: 750), styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false; window.contentViewController = shell
            defer { window.contentViewController = nil; window.close() }
            let (source, original) = try openFixture(tab.controller, watermark: stage == 0)
            var services = PDFWatermarkDependencies()
            if stage == 2 {
                services.runner = { _, _, flag, _ in
                    while !flag.isCancelled { try await Task.sleep(nanoseconds: 1_000_000) }
                    throw OrganizeError.cancelled
                }
            }
            let configuredServices = services
            let task = Task { try await tab.controller.performWatermarkRemoval(dependencies: configuredServices, window: window) }
            try await waitUntil { window.attachedSheet != nil && tab.controller.watermarkJob != nil }
            XCTAssertNotNil(window.attachedSheet, "Native stage \(stage) must present its actual owned sheet")
            XCTAssertFalse(shell.close(tabID: tab.id))
            try await waitUntil { !shell.documentTabs.contains { $0 === tab } }
            _ = try? await task.value
            XCTAssertTrue(shell.documentTabs.isEmpty); XCTAssertNil(window.attachedSheet)
            XCTAssertFalse(tab.controller.workspace.canUndo); XCTAssertFalse(tab.controller.workspace.isDirty)
            XCTAssertEqual(try Data(contentsOf: source), original); XCTAssertFalse(LocalToolActivity.hasRunningJobs)
        }
    }
    func testUnknownSheetAndOtherToolAreNotDismissedOrCanceled() async throws {
        let first = shell(), delegate = AppDelegate(controller: first), oldMenu = NSApp.mainMenu
        delegate.applicationDidFinishLaunching(Notification(name: NSApplication.didFinishLaunchingNotification))
        let window = try XCTUnwrap(delegate.windowSessions.first?.window), tab = first.createBlankDocument()
        defer { window.delegate = nil; window.contentViewController = nil; window.close(); NSApp.mainMenu = oldMenu }
        let task = slowCommand(tab.controller)
        try await waitUntil { tab.controller.watermarkJob != nil }
        let foreign = NSAlert(); foreign.messageText = "Unrelated sheet"; foreign.addButton(withTitle: "Keep")
        foreign.beginSheetModal(for: window) { _ in }
        let approved = await delegate.prepareToCloseWindow(window)
        XCTAssertFalse(approved); XCTAssertTrue(window.attachedSheet === foreign.window)
        _ = try? await task.value
        let other = LocalToolCancellation(); tab.controller.localToolCancellation = other
        let quit = await delegate.prepareToTerminate()
        XCTAssertFalse(quit); XCTAssertFalse(other.isCancelled); XCTAssertTrue(window.attachedSheet === foreign.window)
        tab.controller.localToolCancellation = nil; window.endSheet(foreign.window)
        XCTAssertFalse(LocalToolActivity.hasRunningJobs)
    }
    func testReplacedDocumentDuringCancellationDoesNotCloseTheReplacementTab() async throws {
        let shell = shell(), tab = shell.createBlankDocument()
        let dependencies = PDFWatermarkDependencies(notify: { _, _ in }, runner: { _, _, flag, _ in
            while !flag.isCancelled { try await Task.sleep(nanoseconds: 1_000_000) }
            try await Task.sleep(nanoseconds: 50_000_000)
            throw OrganizeError.cancelled
        })
        let task = Task { try await tab.controller.performWatermarkRemoval(dependencies: dependencies) }
        try await waitUntil { tab.controller.watermarkJob != nil }
        let job = try XCTUnwrap(tab.controller.watermarkJob)
        let close = Task { await shell.closeAfterCancellingWatermark(tab) }
        try await waitUntil { job.cancellation.isCancelled }
        try tab.controller.workspace.createBlank()
        let allowed = await close.value
        _ = try? await task.value
        XCTAssertFalse(allowed); XCTAssertTrue(shell.documentTabs.contains { $0 === tab })
    }
    func testForeignChildOnOwnedConfirmationIsNeverDismissedToForceClosing() async throws {
        let first = shell(), delegate = AppDelegate(controller: first), oldMenu = NSApp.mainMenu
        delegate.applicationDidFinishLaunching(Notification(name: NSApplication.didFinishLaunchingNotification))
        let window = try XCTUnwrap(delegate.windowSessions.first?.window), tab = first.createBlankDocument()
        defer { window.delegate = nil; window.contentViewController = nil; window.close(); NSApp.mainMenu = oldMenu }
        _ = try openFixture(tab.controller, watermark: true)
        let task = Task { try await tab.controller.performWatermarkRemoval(dependencies: PDFWatermarkDependencies(), window: window) }
        try await waitUntil { window.attachedSheet != nil }
        let owned = try XCTUnwrap(window.attachedSheet), job = try XCTUnwrap(tab.controller.watermarkJob)
        let foreign = NSAlert(); foreign.messageText = "Unrelated child"; foreign.addButton(withTitle: "Keep")
        foreign.beginSheetModal(for: owned) { _ in }
        let blocked = await delegate.prepareToCloseWindow(window)
        XCTAssertFalse(blocked)
        XCTAssertFalse(job.cancellation.isCancelled)
        XCTAssertTrue(window.attachedSheet === owned); XCTAssertTrue(owned.attachedSheet === foreign.window)
        owned.endSheet(foreign.window)
        try await waitUntil { owned.attachedSheet == nil }
        let approved = await delegate.prepareToCloseWindow(window)
        XCTAssertTrue(approved)
        _ = try? await task.value
        XCTAssertNil(window.attachedSheet); XCTAssertFalse(LocalToolActivity.hasRunningJobs)
    }
}
