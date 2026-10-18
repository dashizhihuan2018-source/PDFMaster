import AppKit
import Combine
import PaperCore
import UniformTypeIdentifiers

/// One document snapshot, one serial worker. Sizes only use completed engine results.
@MainActor final class PDFCompressionSession {
    enum Status {
        case waiting, processing, ready(PDFCompressionPreview), failed(String)
        var isFailure: Bool { if case .failed = self { return true }; return false }
        var label: String {
            switch self {
            case .waiting: return "等待计算"
            case .processing: return "正在计算实际大小…"
            case .failed: return "计算失败 · 可重试"
            case .ready(let preview):
                let size = ByteCountFormatter.string(fromByteCount: Int64(preview.after), countStyle: .file)
                guard preview.reduced else { return size + " · 无进一步减小" }
                return String(format: "%@ · 减少 %.1f%%", size, 100 * Double(preview.before - preview.after) / Double(preview.before))
            }
        }
    }
    static let qualityPresets: [PDFCompressionPreset] = [.smallSize, .balanced, .highQuality]
    let input: PDFEngineInput
    private(set) var selectedPreset: PDFCompressionPreset = .balanced
    private(set) var isClosed = false
    private(set) var isStale = false
    private(set) var message = "正在逐档计算实际大小；选择任一档查看结果。"
    private(set) var exportFeedback: String?
    private(set) var exportFailed = false
    var onChange: (() -> Void)?
    private var statuses: [PDFCompressionPreset: Status] = [:]
    private var queue: [PDFCompressionPreset] = []
    private var active: PDFCompressionPreset?
    private var task: Task<Void, Never>?
    private var generation = 0
    private let worker: (PDFCompressionPreset) async throws -> PDFCompressionPreview
    private let cancelWorker: () -> Void
    private let validate: () throws -> Void

    init(input: PDFEngineInput,
         worker: @escaping (PDFCompressionPreset) async throws -> PDFCompressionPreview,
         cancelWorker: @escaping () -> Void = {}, validate: @escaping () throws -> Void = {}) {
        self.input = input; self.worker = worker; self.cancelWorker = cancelWorker; self.validate = validate
    }
    func status(for preset: PDFCompressionPreset) -> Status { statuses[preset] ?? .waiting }
    var selectedPreview: PDFCompressionPreview? {
        guard !isClosed, !isStale, case .ready(let preview) = status(for: selectedPreset) else { return nil }
        return preview
    }
    var canExport: Bool { selectedPreview?.reduced == true && (try? validate()) != nil }
    var isProcessing: Bool { active != nil && !isClosed && !isStale }

    func start() {
        guard !isClosed, !isStale, task == nil, statuses.isEmpty else { return }
        queue = [.balanced, .highQuality, .smallSize]; advance()
    }
    func select(_ preset: PDFCompressionPreset) {
        guard !isClosed, !isStale else { return }
        if selectedPreset != preset { exportFeedback = nil; exportFailed = false }
        selectedPreset = preset
        if case .waiting = status(for: preset), active != preset {
            queue.removeAll { $0 == preset }; queue.insert(preset, at: 0)
        }
        onChange?(); advance()
    }
    func retrySelected() {
        guard !isClosed, !isStale, status(for: selectedPreset).isFailure else { return }
        statuses[selectedPreset] = .waiting
        queue.removeAll { $0 == selectedPreset }; queue.insert(selectedPreset, at: 0)
        onChange?(); advance()
    }
    @discardableResult func saveCopy(to destination: URL, export: (PDFCompressionPreview, URL) throws -> Void) -> Bool {
        checkFreshness()
        guard canExport, let preview = selectedPreview else { return false }
        do {
            try export(preview, destination)
            exportFailed = false; exportFeedback = "压缩副本已保存 · " + destination.lastPathComponent
            onChange?(); return true
        } catch {
            exportFailed = true
            exportFeedback = error as? OrganizeError == .outputExists
                ? "未保存：该文件已存在。请再次另存并选择新的文件名。"
                : "未保存，可重试：" + WorkbenchController.localToolError(error)
            onChange?(); return false
        }
    }
    func checkFreshness() {
        guard !isClosed, !isStale else { return }
        do { try validate() }
        catch { invalidate() }
    }
    private func invalidate() {
        guard !isStale, !isClosed else { return }
        isStale = true; generation += 1; queue.removeAll(); statuses.removeAll()
        message = "文档已变化，压缩结果已过期。请关闭后重新打开压缩面板。"
        if active != nil { cancelWorker(); task?.cancel() }
        onChange?()
    }
    func close() {
        guard !isClosed else { return }
        isClosed = true; generation += 1; queue.removeAll(); statuses.removeAll()
        if active != nil && !isStale { cancelWorker(); task?.cancel() }
        onChange?(); onChange = nil
    }
    private func advance() {
        guard active == nil, !isClosed, !isStale, !queue.isEmpty else { return }
        checkFreshness(); guard !isStale else { return }
        let preset = queue.removeFirst(), token = generation
        active = preset; statuses[preset] = .processing; onChange?()
        task = Task { [weak self, worker] in
            let outcome: Result<PDFCompressionPreview, Error>
            do { outcome = .success(try await worker(preset)) }
            catch { outcome = .failure(error) }
            guard let self, !self.isClosed, !self.isStale, self.generation == token else { return }
            self.active = nil; self.task = nil
            self.checkFreshness(); guard !self.isStale else { return }
            switch outcome {
            case .success(let result): self.statuses[preset] = .ready(result)
            case .failure(let error): self.statuses[preset] = .failed(WorkbenchController.localToolError(error))
            }
            self.onChange?(); self.advance()
        }
    }
}

/// Modeless, document-owned panel: the main window remains available to external opens.
@MainActor final class PDFCompressionPanelController: NSWindowController, NSWindowDelegate {
    let session: PDFCompressionSession
    var onClose: (() -> Void)?
    private weak var workbench: WorkbenchController?
    private weak var parent: NSWindow?
    private let sourceName: String
    private let sourceURL: URL?
    private let qualitySelector = NSSegmentedControl(labels: ["紧凑 · 72 DPI", "均衡 · 150 DPI", "高清 · 300 DPI"], trackingMode: .selectOne, target: nil, action: nil)
    private let progress = NSProgressIndicator()
    private let statusLabel = NSTextField(wrappingLabelWithString: "")
    private let resultLabel = NSTextField(labelWithString: "")
    private let exportButton = NSButton(title: "另存为压缩副本…", target: nil, action: nil)
    private let retryButton = NSButton(title: "重试此档", target: nil, action: nil)
    private let losslessButton = NSButton(checkboxWithTitle: "仅无损优化（不降低画质）", target: nil, action: nil)
    private var observations = Set<AnyCancellable>()
    private var parentCloseObserver: NSObjectProtocol?
    private var savePanel: NSSavePanel?
    private var didFinish = false

    init(workbench: WorkbenchController, parent: NSWindow, qpdf: URL) throws {
        let input = try workbench.workspace.engineInput()
        sourceURL = workbench.workspace.sourceURL
        sourceName = sourceURL?.lastPathComponent ?? "未命名 PDF"
        self.workbench = workbench; self.parent = parent
        session = PDFCompressionSession(input: input, worker: { [weak workbench] preset in
            guard let workbench else { throw OrganizeError.cancelled }
            return try await workbench.prepareCompression(preset: preset, qpdf: qpdf)
        }, cancelWorker: { [weak workbench] in workbench?.cancelLocalTool() }, validate: { [weak workbench] in
            guard let workbench else { throw OrganizeError.cancelled }
            try workbench.workspace.validateEngineInput(input)
        })
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 520, height: 360),
                            styleMask: [.titled, .closable], backing: .buffered, defer: false)
        panel.title = "压缩文件大小"
        panel.isReleasedWhenClosed = false; panel.hidesOnDeactivate = false
        super.init(window: panel)
        panel.delegate = self
        buildInterface()
        session.onChange = { [weak self] in self?.render() }
        // Published callbacks are delivered before mutations; validate on the following main turn.
        workbench.workspace.$revision.dropFirst().sink { [weak self] _ in self?.scheduleFreshnessCheck() }.store(in: &observations)
        workbench.workspace.$document.dropFirst().sink { [weak self] _ in self?.scheduleFreshnessCheck() }.store(in: &observations)
        parentCloseObserver = NotificationCenter.default.addObserver(forName: NSWindow.willCloseNotification, object: parent, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.close() }
        }
        render()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func present() {
        guard !session.isClosed else { return }
        if let parent, let window {
            let x = parent.frame.midX - window.frame.width / 2, y = parent.frame.midY - window.frame.height / 2
            window.setFrameOrigin(NSPoint(x: x, y: y))
            window.setFrame(window.constrainFrameRect(window.frame, to: parent.screen), display: false)
        }
        showWindow(nil); window?.makeKeyAndOrderFront(nil); session.start()
    }
    private func scheduleFreshnessCheck() {
        Task { [weak self] in self?.session.checkFreshness() }
    }
    override func close() { finish(); super.close() }
    func windowWillClose(_ notification: Notification) { finish() }
    private func finish() {
        guard !didFinish else { return }; didFinish = true
        session.close(); observations.removeAll()
        if let token = parentCloseObserver { NotificationCenter.default.removeObserver(token); parentCloseObserver = nil }
        if let savePanel, let window, savePanel.sheetParent === window { window.endSheet(savePanel, returnCode: .cancel) }
        savePanel = nil
        let completion = onClose; onClose = nil; completion?()
    }

    private func buildInterface() {
        guard let content = window?.contentView else { return }
        let title = label("压缩文件大小", size: 18, weight: .semibold)
        let subtitle = label(sourceName, size: 12, color: .secondaryLabelColor)
        subtitle.lineBreakMode = .byTruncatingMiddle
        subtitle.identifier = NSUserInterfaceItemIdentifier("CompressionSourceName")
        let qualityLabel = label("文档质量", size: 12, weight: .medium)
        qualitySelector.selectedSegment = 1
        qualitySelector.target = self; qualitySelector.action = #selector(selectQuality)
        qualitySelector.identifier = NSUserInterfaceItemIdentifier("CompressionQualitySelector")
        qualitySelector.setAccessibilityLabel("文档质量")
        qualitySelector.segmentStyle = .rounded
        for index in 0..<qualitySelector.segmentCount { qualitySelector.setWidth(156, forSegment: index) }
        losslessButton.target = self; losslessButton.action = #selector(selectLossless)
        losslessButton.font = .systemFont(ofSize: 11)
        losslessButton.identifier = NSUserInterfaceItemIdentifier("CompressionLossless")
        resultLabel.font = .monospacedDigitSystemFont(ofSize: 16, weight: .medium)
        resultLabel.identifier = NSUserInterfaceItemIdentifier("CompressionSizeResult")
        resultLabel.setAccessibilityLabel("实际文件大小")
        progress.style = .spinning; progress.isIndeterminate = true; progress.controlSize = .small
        progress.widthAnchor.constraint(equalToConstant: 18).isActive = true
        progress.heightAnchor.constraint(equalToConstant: 18).isActive = true
        statusLabel.font = .systemFont(ofSize: 11)
        statusLabel.identifier = NSUserInterfaceItemIdentifier("CompressionStatus")
        statusLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        retryButton.target = self; retryButton.action = #selector(retry)
        retryButton.identifier = NSUserInterfaceItemIdentifier("CompressionRetry")
        let statusRow = NSStackView(views: [progress, statusLabel, retryButton]); statusRow.spacing = 10
        let hint = NSTextField(wrappingLabelWithString: "仅本机处理，原件与当前编辑保持不变。复杂色彩、透明度和遮罩图片保守处理，实际收益可能有限。")
        hint.font = .systemFont(ofSize: 11); hint.textColor = .secondaryLabelColor
        let cancel = NSButton(title: "取消", target: self, action: #selector(cancel)); cancel.keyEquivalent = "\u{1b}"
        exportButton.target = self; exportButton.action = #selector(saveCopy); exportButton.keyEquivalent = "\r"
        exportButton.bezelStyle = .rounded; exportButton.identifier = NSUserInterfaceItemIdentifier("CompressionSaveAs")
        let footer = NSStackView(views: [NSView(), cancel, exportButton]); footer.spacing = 10
        for view in [title, subtitle, qualityLabel, qualitySelector, losslessButton, resultLabel, statusRow, hint, footer] {
            view.translatesAutoresizingMaskIntoConstraints = false; content.addSubview(view)
            view.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 24).isActive = true
            view.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -24).isActive = true
        }
        NSLayoutConstraint.activate([
            title.topAnchor.constraint(equalTo: content.topAnchor, constant: 22), title.heightAnchor.constraint(equalToConstant: 22),
            subtitle.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 6), subtitle.heightAnchor.constraint(equalToConstant: 16),
            qualityLabel.topAnchor.constraint(equalTo: subtitle.bottomAnchor, constant: 18), qualityLabel.heightAnchor.constraint(equalToConstant: 16),
            qualitySelector.topAnchor.constraint(equalTo: qualityLabel.bottomAnchor, constant: 8), qualitySelector.heightAnchor.constraint(equalToConstant: 28),
            losslessButton.topAnchor.constraint(equalTo: qualitySelector.bottomAnchor, constant: 8), losslessButton.heightAnchor.constraint(equalToConstant: 18),
            resultLabel.topAnchor.constraint(equalTo: losslessButton.bottomAnchor, constant: 16), resultLabel.heightAnchor.constraint(equalToConstant: 24),
            statusRow.topAnchor.constraint(equalTo: resultLabel.bottomAnchor, constant: 6), statusRow.heightAnchor.constraint(equalToConstant: 36),
            hint.topAnchor.constraint(equalTo: statusRow.bottomAnchor, constant: 8), hint.heightAnchor.constraint(equalToConstant: 30),
            footer.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -18), footer.heightAnchor.constraint(equalToConstant: 32)
        ])
    }
    private func render() {
        guard !didFinish else { return }
        if let index = PDFCompressionSession.qualityPresets.firstIndex(of: session.selectedPreset) {
            qualitySelector.selectedSegment = index
        }
        qualitySelector.isEnabled = !session.isStale
        losslessButton.state = session.selectedPreset == .lossless ? .on : .off
        losslessButton.isEnabled = !session.isStale
        let selected = session.status(for: session.selectedPreset)
        let original = bytes(session.input.data.count)
        resultLabel.stringValue = original + " → " + (session.isStale ? "结果已过期" : selected.label)
        if session.isStale { statusLabel.stringValue = session.message }
        else if let feedback = session.exportFeedback { statusLabel.stringValue = feedback }
        else if case .failed(let error) = selected { statusLabel.stringValue = error }
        else if let preview = session.selectedPreview { statusLabel.stringValue = preview.reduced ? "实际大小已计算，可另存为新副本。" : "此档未能进一步缩小文件，原件保持不变。" }
        else { statusLabel.stringValue = "正在本机计算所选质量的实际大小，请稍候。" }
        statusLabel.textColor = session.isStale || session.exportFailed || selected.isFailure ? .systemRed : .secondaryLabelColor
        retryButton.isHidden = !selected.isFailure || session.isStale
        exportButton.isEnabled = session.canExport && savePanel == nil
        if session.isProcessing { progress.startAnimation(nil); progress.isHidden = false }
        else { progress.stopAnimation(nil); progress.isHidden = true }
    }
    @objc private func selectQuality() {
        guard PDFCompressionSession.qualityPresets.indices.contains(qualitySelector.selectedSegment) else { return }
        session.select(PDFCompressionSession.qualityPresets[qualitySelector.selectedSegment])
    }
    @objc private func selectLossless() {
        if losslessButton.state == .on { session.select(.lossless) }
        else { selectQuality() }
    }
    @objc private func retry() { session.retrySelected() }
    @objc private func cancel() { close() }
    @objc private func saveCopy() {
        session.checkFreshness()
        guard !session.isClosed, session.canExport, let window, savePanel == nil else { return }
        let panel = NSSavePanel(); panel.allowedContentTypes = [.pdf]; panel.canCreateDirectories = true
        panel.title = "另存为压缩副本"; panel.message = "请选择新的文件名；不会覆盖原文件或任何现有文件。"
        panel.nameFieldStringValue = (sourceURL?.deletingPathExtension().lastPathComponent ?? "未命名") + "-压缩.pdf"
        panel.directoryURL = sourceURL?.deletingLastPathComponent()
        savePanel = panel; render()
        panel.beginSheetModal(for: window) { [weak self] response in
            guard let self else { return }
            self.savePanel = nil
            guard !self.session.isClosed, !self.didFinish else { return }
            self.session.checkFreshness()
            guard response == .OK, let url = panel.url, self.session.canExport else { self.render(); return }
            self.session.saveCopy(to: url) { [weak self] preview, destination in
                guard let workbench = self?.workbench else { throw OrganizeError.cancelled }
                try workbench.exportCompression(preview, to: destination)
            }
        }
    }
    private func label(_ text: String, size: CGFloat, weight: NSFont.Weight = .regular, color: NSColor = .labelColor) -> NSTextField {
        let field = NSTextField(labelWithString: text); field.font = .systemFont(ofSize: size, weight: weight); field.textColor = color; return field
    }
    private func bytes(_ count: Int) -> String { ByteCountFormatter.string(fromByteCount: Int64(count), countStyle: .file) }
}
