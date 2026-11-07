import AppKit
import PDFKit
import PaperCore

@MainActor enum PDFWatermarkAdmissionBoundary {
    static func prepare<T>(sources: [Data], validate: (Data) async throws -> Void, serialize: () throws -> T) async throws -> T {
        for source in sources { try await validate(source) }
        return try serialize()
    }
}

enum PDFWatermarkPhase { case admission, scan, removal }
enum PDFWatermarkOutcome: Equatable { case noCandidates, cancelled, removed(Int) }
typealias PDFWatermarkOperation = (URL, URL, LocalToolCancellation) throws -> String
typealias PDFWatermarkJobRunner = @MainActor (PDFWatermarkPhase, Data, LocalToolCancellation, @escaping PDFWatermarkOperation) async throws -> LocalToolResult

/// Ownership and lifetime of one ordinary command, including its modal waits.
/// Closing a tab/window may dismiss only this command's exact attached sheet.
@MainActor final class PDFWatermarkJob {
    let cancellation = LocalToolCancellation()
    private var sheet: NSWindow?
    private weak var parent: NSWindow?
    private var completed = false
    private var waiters: [CheckedContinuation<Void, Never>] = []
    var canCancelForClosing: Bool {
        guard let sheet else { return true }
        guard sheet.attachedSheet == nil else { return false }
        guard let parent, let attached = parent.attachedSheet else { return true }
        return attached === sheet
    }
    func own(_ sheet: NSWindow, parent: NSWindow) { self.sheet = sheet; self.parent = parent }
    func release(_ sheet: NSWindow) { if self.sheet === sheet { self.sheet = nil; parent = nil } }
    func cancel() {
        cancellation.cancel()
        if let sheet, sheet.attachedSheet == nil, let parent, parent.attachedSheet === sheet {
            parent.endSheet(sheet, returnCode: .abort)
        }
    }
    func waitUntilFinished() async {
        guard !completed else { return }
        await withCheckedContinuation { waiters.append($0) }
    }
    func finish() {
        completed = true
        let pending = waiters; waiters.removeAll()
        pending.forEach { $0.resume() }
    }
}

/// Native interaction and worker boundaries for the current-document command.
/// No tool page, preview/export workflow, AI service or source-file write.
@MainActor struct PDFWatermarkDependencies {
    private var confirmation: (@MainActor (NSWindow?, Int) async -> Bool)?
    private var notice: (@MainActor (NSWindow?, String) async -> Void)?
    private var progressPresentation: (@MainActor (NSWindow?, String, @escaping () -> Void) -> (() -> Void))?
    var ownedJob: PDFWatermarkJob?
    var runner: PDFWatermarkJobRunner
    let progressDelayNanoseconds: UInt64 = 500_000_000
    init(confirm: (@MainActor (NSWindow?, Int) async -> Bool)? = nil,
         notify: (@MainActor (NSWindow?, String) async -> Void)? = nil,
         progress: (@MainActor (NSWindow?, String, @escaping () -> Void) -> (() -> Void))? = nil,
         runner: @escaping PDFWatermarkJobRunner = { _, data, flag, operation in
             try await LocalToolJob.run(data: data, cancellation: flag, operation: operation)
         }) {
        self.confirmation = confirm; self.notice = notify; self.progressPresentation = progress; self.runner = runner
    }
    func confirm(_ window: NSWindow?, _ count: Int) async -> Bool {
        if let confirmation { return await confirmation(window, count) }
        return await Self.nativeConfirm(window, count, job: ownedJob)
    }
    func notify(_ window: NSWindow?, _ message: String) async {
        if let notice { await notice(window, message) }
        else { await Self.nativeNotice(window, message, job: ownedJob) }
    }
    private static func nativeConfirm(_ window: NSWindow?, _ count: Int, job: PDFWatermarkJob?) async -> Bool {
        guard let window, job?.cancellation.isCancelled != true else { return false }
        let alert = NSAlert(); alert.messageText = "确定要删除已有水印吗？"
        alert.informativeText = "仅删除可识别的标准水印。此修改可撤销，文件不会自动保存。"
        alert.addButton(withTitle: "确定"); alert.addButton(withTitle: "取消")
        return await withCheckedContinuation { continuation in
            alert.beginSheetModal(for: window) { response in
                job?.release(alert.window)
                continuation.resume(returning: response == .alertFirstButtonReturn)
            }
            job?.own(alert.window, parent: window)
        }
    }
    private static func nativeNotice(_ window: NSWindow?, _ message: String, job: PDFWatermarkJob?) async {
        guard let window, job?.cancellation.isCancelled != true else { return }
        let alert = NSAlert(); alert.messageText = "删除水印"; alert.informativeText = message; alert.addButton(withTitle: "好")
        await withCheckedContinuation { continuation in
            alert.beginSheetModal(for: window) { _ in job?.release(alert.window); continuation.resume() }
            job?.own(alert.window, parent: window)
        }
    }
    private static func nativeProgress(_ window: NSWindow?, _ title: String, _ cancel: @escaping () -> Void, job: PDFWatermarkJob?) -> (() -> Void) {
        guard let window, window.attachedSheet == nil else { return {} }
        let alert = NSAlert(); alert.messageText = title; alert.informativeText = "仅在本机处理当前文档。取消不会应用未完成的修改。"
        alert.addButton(withTitle: "取消")
        let indicator = NSProgressIndicator(frame: NSRect(x: 0, y: 0, width: 300, height: 14))
        indicator.style = .bar; indicator.isIndeterminate = true; indicator.startAnimation(nil); alert.accessoryView = indicator
        alert.beginSheetModal(for: window) { response in job?.release(alert.window); if response == .alertFirstButtonReturn { cancel() } }
        job?.own(alert.window, parent: window)
        return {
            indicator.stopAnimation(nil)
            if window.attachedSheet === alert.window { window.endSheet(alert.window, returnCode: .stop) }
            job?.release(alert.window)
        }
    }
    func run(_ phase: PDFWatermarkPhase, data: Data, cancellation: LocalToolCancellation, window: NSWindow?, operation: @escaping PDFWatermarkOperation) async throws -> LocalToolResult {
        guard !cancellation.isCancelled else { throw OrganizeError.cancelled }
        var dismiss: (() -> Void)?
        let delayed = Task { @MainActor in
            try await Task.sleep(nanoseconds: progressDelayNanoseconds)
            guard !Task.isCancelled, !cancellation.isCancelled else { return }
            let title = phase == .removal ? "正在删除水印…" : "正在检测水印…"
            if let progressPresentation { dismiss = progressPresentation(window, title, { cancellation.cancel() }) }
            else { dismiss = Self.nativeProgress(window, title, { cancellation.cancel() }, job: ownedJob) }
        }
        defer { delayed.cancel(); dismiss?() }
        let result = try await runner(phase, data, cancellation, operation)
        guard !cancellation.isCancelled else { throw OrganizeError.cancelled }
        return result
    }
}

@MainActor extension WorkbenchController {
    @objc func removeWatermark(_ sender: Any?) { presentWatermarkRemoval() }
    func presentWatermarkRemoval() {
        guard let window = view.window, !isLocalToolRunning, window.attachedSheet == nil,
              pdfView.finishInlineTextEditing(), workspace.document != nil else { return }
        let dependencies = watermarkDependencies
        Task { [weak self, weak window] in
            guard let self, let window else { return }
            do { _ = try await self.performWatermarkRemoval(dependencies: dependencies, window: window) }
            catch {
                let message = Self.localToolError(error)
                self.setInteractionStatus(message)
                if error as? OrganizeError != .cancelled { await dependencies.notify(window, message) }
            }
        }
    }

    /// WPS ordinary-command behavior: scan current document, confirm once,
    /// commit one undoable edit. Persist only through the normal Save chain.
    func performWatermarkRemoval(dependencies: PDFWatermarkDependencies, window: NSWindow? = nil) async throws -> PDFWatermarkOutcome {
        guard !isLocalToolRunning else { throw OrganizeError.unavailable }
        guard let document = workspace.document else { throw PDFWorkspaceError.noDocument }
        guard !workspace.isEncryptedSource, workspace.allowsCopying, workspace.allowsDocumentAssembly else { throw PDFWorkspaceError.permissionDenied }
        let job = PDFWatermarkJob(), flag = job.cancellation
        watermarkJob = job; localToolCancellation = flag
        var dependencies = dependencies; dependencies.ownedJob = job
        let activity = LocalToolActivity.register(flag)
        defer {
            if localToolCancellation === flag { localToolCancellation = nil }
            if watermarkJob === job { watermarkJob = nil }
            LocalToolActivity.finish(activity)
            job.finish()
        }
        let revision = workspace.revision, identity = ObjectIdentifier(document), engine = PDFWatermarkEngine()
        setInteractionStatus("正在检测当前文档水印…")
        let input: PDFEngineInput = try await PDFWatermarkAdmissionBoundary.prepare(sources: workspace.engineAdmissionDatas, validate: { raw in
            _ = try await dependencies.run(.admission, data: raw, cancellation: flag, window: window) { source, output, cancellation in
                try engine.validateAdmission(raw, in: source.deletingLastPathComponent(), cancelled: { cancellation.isCancelled })
                try FileManager.default.copyItem(at: source, to: output)
                return ""
            }
        }, serialize: {
            guard !flag.isCancelled else { throw OrganizeError.cancelled }
            guard self.workspace.revision == revision, self.workspace.document.map(ObjectIdentifier.init) == identity else { throw PDFWorkspaceError.staleEngineResult }
            return try self.workspace.engineInput()
        })
        let pages = Set(0..<document.pageCount)
        let inspected = try await dependencies.run(.scan, data: input.data, cancellation: flag, window: window) { source, output, cancellation in
            let scan = try engine.scan(source, pages: pages, cancelled: { cancellation.isCancelled })
            try FileManager.default.copyItem(at: source, to: output)
            return String(decoding: try JSONEncoder().encode(scan), as: UTF8.self)
        }
        try workspace.validateEngineInput(input)
        let scan = try JSONDecoder().decode(PDFWatermarkScan.self, from: Data(inspected.message.utf8))
        guard !scan.candidates.isEmpty else {
            let message = "未检测到可直接删除的水印。若页面上仍能看到水印，可能是图片或扫描痕迹，暂不支持。"
            setInteractionStatus(message); await dependencies.notify(window, message)
            guard !flag.isCancelled else { throw OrganizeError.cancelled }
            return .noCandidates
        }
        guard await dependencies.confirm(window, scan.candidates.count) else {
            setInteractionStatus("已取消删除水印 · 文档未修改")
            return .cancelled
        }
        guard !flag.isCancelled else { throw OrganizeError.cancelled }
        try workspace.validateEngineInput(input)
        let result = try await dependencies.run(.removal, data: input.data, cancellation: flag, window: window) { source, output, cancellation in
            try engine.remove(source, scan: scan, selectedIDs: Set(scan.candidates.map(\.id)), output: output, cancelled: { cancellation.isCancelled })
            return ""
        }
        guard !flag.isCancelled else { throw OrganizeError.cancelled }
        let visiblePage = pdfView.currentPage.flatMap { pdfView.document?.index(for: $0) } ?? 0
        let page = visiblePage == NSNotFound ? 0 : visiblePage, scale = pdfView.scaleFactor
        try workspace.acceptEngineResult(result.data, for: input)
        refreshDocument(preservePage: page, scale: scale)
        setInteractionStatus("水印已删除 · 可撤销 · 尚未保存")
        return .removed(scan.candidates.count)
    }
}
