import AppKit
import Combine
import PDFKit
import PaperCore
import UniformTypeIdentifiers

/// One document snapshot, one serial worker. Card labels only use completed engine results.
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
    static let cardPresets: [PDFCompressionPreset] = [.highQuality, .balanced, .smallSize]
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
        message = "文档已变化，此预览已过期。请关闭后重新打开压缩面板。"
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
    private let pdfView = PDFView()
    private let previewMode = NSSegmentedControl(labels: ["原始", "压缩后"], trackingMode: .selectOne, target: nil, action: nil)
    private let progress = NSProgressIndicator()
    private let statusLabel = NSTextField(wrappingLabelWithString: "")
    private let previewLabel = NSTextField(labelWithString: "原始预览")
    private let exportButton = NSButton(title: "另存为压缩副本…", target: nil, action: nil)
    private let retryButton = NSButton(title: "重试此档", target: nil, action: nil)
    private let losslessButton = NSButton(radioButtonWithTitle: "无损优化 · 不降低图片画质", target: nil, action: nil)
    private let losslessResult = NSTextField(labelWithString: "选择后计算实际大小")
    private var cards: [PDFCompressionPreset: CompressionPresetCard] = [:]
    private var observations = Set<AnyCancellable>()
    private var parentCloseObserver: NSObjectProtocol?
    private var savePanel: NSSavePanel?
    private var displayedData: Data?
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
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 1020, height: 700),
                            styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        panel.title = "压缩文件大小"; panel.minSize = NSSize(width: 960, height: 650)
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
        savePanel = nil; pdfView.document = nil; displayedData = nil
        let completion = onClose; onClose = nil; completion?()
    }

    private func buildInterface() {
        guard let content = window?.contentView else { return }
        let title = label("压缩文件大小", size: 23, weight: .semibold)
        let subtitle = label("\(sourceName)  ·  原始 \(bytes(session.input.data.count))", size: 12, color: .secondaryLabelColor)
        subtitle.lineBreakMode = .byTruncatingMiddle
        let left = NSStackView(); left.orientation = .vertical; left.alignment = .leading; left.spacing = 12
        left.addArrangedSubview(label("选择图片质量", size: 14, weight: .semibold))
        for preset in PDFCompressionSession.cardPresets {
            let card = CompressionPresetCard(preset: preset, target: self, action: #selector(selectCard(_:)))
            card.widthAnchor.constraint(equalToConstant: 330).isActive = true
            card.heightAnchor.constraint(equalToConstant: 96).isActive = true
            cards[preset] = card; left.addArrangedSubview(card)
        }
        losslessButton.target = self; losslessButton.action = #selector(selectLossless)
        losslessButton.identifier = NSUserInterfaceItemIdentifier("CompressionLossless")
        left.addArrangedSubview(losslessButton)
        losslessResult.font = .systemFont(ofSize: 11); losslessResult.textColor = .secondaryLabelColor
        left.addArrangedSubview(losslessResult)
        let hint = NSTextField(wrappingLabelWithString: "仅在本机处理，不上传文件。原件与当前编辑保持不变。\n\n仅优化适合处理的图片，保留文字、矢量与批注结构；复杂色彩、透明度、遮罩等图片采用保守处理，实际收益可能有限。")
        hint.font = .systemFont(ofSize: 11); hint.textColor = .secondaryLabelColor
        hint.widthAnchor.constraint(equalToConstant: 330).isActive = true; left.addArrangedSubview(hint)

        previewMode.selectedSegment = 0; previewMode.target = self; previewMode.action = #selector(changePreview)
        previewMode.identifier = NSUserInterfaceItemIdentifier("CompressionPreviewMode")
        let minus = NSButton(title: "−", target: self, action: #selector(zoomOut))
        minus.setAccessibilityLabel("缩小预览")
        let plus = NSButton(title: "+", target: self, action: #selector(zoomIn))
        plus.setAccessibilityLabel("放大预览")
        let fit = NSButton(title: "适合页面", target: self, action: #selector(fitPreview))
        let toolbar = NSStackView(views: [previewMode, NSView(), minus, plus, fit]); toolbar.spacing = 8
        pdfView.displayMode = .singlePageContinuous; pdfView.autoScales = true; pdfView.backgroundColor = .underPageBackgroundColor
        pdfView.minScaleFactor = 0.1; pdfView.maxScaleFactor = 5
        pdfView.identifier = NSUserInterfaceItemIdentifier("CompressionPDFPreview")
        previewLabel.font = .systemFont(ofSize: 11); previewLabel.textColor = .secondaryLabelColor
        let right = NSStackView(views: [toolbar, pdfView, previewLabel]); right.orientation = .vertical; right.alignment = .leading; right.spacing = 10
        toolbar.widthAnchor.constraint(equalTo: right.widthAnchor).isActive = true
        pdfView.widthAnchor.constraint(equalTo: right.widthAnchor).isActive = true
        pdfView.heightAnchor.constraint(greaterThanOrEqualToConstant: 360).isActive = true
        let columns = NSStackView(views: [left, right]); columns.alignment = .top; columns.spacing = 24
        left.widthAnchor.constraint(equalToConstant: 330).isActive = true
        right.heightAnchor.constraint(equalTo: columns.heightAnchor).isActive = true

        progress.style = .spinning; progress.isIndeterminate = true; progress.controlSize = .small
        progress.widthAnchor.constraint(equalToConstant: 18).isActive = true
        statusLabel.font = .systemFont(ofSize: 11)
        statusLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        retryButton.target = self; retryButton.action = #selector(retry)
        let statusRow = NSStackView(views: [progress, statusLabel, retryButton]); statusRow.spacing = 10
        let cancel = NSButton(title: "取消", target: self, action: #selector(cancel)); cancel.keyEquivalent = "\u{1b}"
        exportButton.target = self; exportButton.action = #selector(saveCopy); exportButton.keyEquivalent = "\r"
        exportButton.bezelStyle = .rounded; exportButton.identifier = NSUserInterfaceItemIdentifier("CompressionSaveAs")
        let footer = NSStackView(views: [label("保留原文件 · 仅另存为新副本", size: 11, color: .secondaryLabelColor), NSView(), cancel, exportButton]); footer.spacing = 12
        let root = NSStackView(views: [title, subtitle, columns, statusRow, footer]); root.orientation = .vertical; root.alignment = .leading; root.spacing = 12
        root.translatesAutoresizingMaskIntoConstraints = false; content.addSubview(root)
        for row in [columns, statusRow, footer] { row.widthAnchor.constraint(equalTo: root.widthAnchor).isActive = true }
        NSLayoutConstraint.activate([root.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 24), root.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -24), root.topAnchor.constraint(equalTo: content.topAnchor, constant: 22), root.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -20)])
    }
    private func render() {
        guard !didFinish else { return }
        for (preset, card) in cards {
            card.update(selected: session.selectedPreset == preset, status: session.status(for: preset), originalBytes: session.input.data.count)
            card.isEnabled = !session.isStale
        }
        losslessButton.state = session.selectedPreset == .lossless ? .on : .off
        losslessButton.isEnabled = !session.isStale
        losslessResult.stringValue = session.status(for: .lossless).label
        let selected = session.status(for: session.selectedPreset)
        if session.isStale { statusLabel.stringValue = session.message }
        else if let feedback = session.exportFeedback { statusLabel.stringValue = feedback }
        else if case .failed(let error) = selected { statusLabel.stringValue = error }
        else if let preview = session.selectedPreview { statusLabel.stringValue = preview.summary }
        else { statusLabel.stringValue = session.message }
        statusLabel.textColor = session.isStale || session.exportFailed || selected.isFailure ? .systemRed : .secondaryLabelColor
        retryButton.isHidden = !selected.isFailure || session.isStale
        exportButton.isEnabled = session.canExport && savePanel == nil
        if session.isProcessing { progress.startAnimation(nil); progress.isHidden = false }
        else { progress.stopAnimation(nil); progress.isHidden = true }
        previewMode.setEnabled(session.selectedPreview != nil, forSegment: 1)
        updatePreview()
    }
    private func updatePreview() {
        let compressed = previewMode.selectedSegment == 1
        let data = compressed ? session.selectedPreview?.data : session.input.data
        if displayedData != data {
            let page = pdfView.currentPage.flatMap { pdfView.document?.index(for: $0) } ?? 0
            let scale = pdfView.scaleFactor, auto = pdfView.autoScales
            pdfView.document = data.flatMap(PDFDocument.init(data:)); displayedData = data
            if let document = pdfView.document, let target = document.page(at: min(page, max(0, document.pageCount - 1))) { pdfView.go(to: target) }
            pdfView.autoScales = auto; if !auto { pdfView.scaleFactor = scale }
        }
        previewLabel.stringValue = compressed
            ? (session.selectedPreview.map { "压缩预览 · \(bytes($0.after)) · \(pdfView.document?.pageCount ?? 0) 页" } ?? "此档尚无可用结果；请等待计算或重试。")
            : "原始快照 · \(bytes(session.input.data.count)) · \(pdfView.document?.pageCount ?? 0) 页"
    }
    @objc private func selectCard(_ sender: CompressionPresetCard) { session.select(sender.preset) }
    @objc private func selectLossless() { session.select(.lossless) }
    @objc private func changePreview() { session.checkFreshness(); updatePreview() }
    @objc private func zoomIn() { pdfView.autoScales = false; pdfView.zoomIn(nil) }
    @objc private func zoomOut() { pdfView.autoScales = false; pdfView.zoomOut(nil) }
    @objc private func fitPreview() { pdfView.autoScales = true }
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

@MainActor private final class CompressionPresetCard: NSButton {
    let preset: PDFCompressionPreset
    private let name = NSTextField(labelWithString: "")
    private let detail = NSTextField(labelWithString: "")
    private let result = NSTextField(labelWithString: "等待计算")
    private let spinner = NSProgressIndicator()
    init(preset: PDFCompressionPreset, target: AnyObject, action: Selector) {
        self.preset = preset
        super.init(frame: .zero)
        self.target = target; self.action = action; title = ""; isBordered = false
        wantsLayer = true; layer?.cornerRadius = 10; layer?.borderWidth = 1.5
        identifier = NSUserInterfaceItemIdentifier("ReduceFileSizeQualityButton-\(preset.imageDPI ?? 0)")
        let names: [PDFCompressionPreset: String] = [.highQuality: "高清", .balanced: "均衡", .smallSize: "紧凑"]
        name.stringValue = "\(names[preset] ?? preset.title)  ·  \(preset.imageDPI ?? 0) DPI"
        name.font = .systemFont(ofSize: 14, weight: .semibold)
        detail.font = .systemFont(ofSize: 10); detail.textColor = .secondaryLabelColor
        result.font = .monospacedDigitSystemFont(ofSize: 11, weight: .medium)
        spinner.style = .spinning; spinner.controlSize = .small; spinner.isIndeterminate = true
        for view in [name, detail, result, spinner] { view.translatesAutoresizingMaskIntoConstraints = false; addSubview(view) }
        NSLayoutConstraint.activate([name.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 15), name.topAnchor.constraint(equalTo: topAnchor, constant: 12), detail.leadingAnchor.constraint(equalTo: name.leadingAnchor), detail.topAnchor.constraint(equalTo: name.bottomAnchor, constant: 7), result.leadingAnchor.constraint(equalTo: name.leadingAnchor), result.topAnchor.constraint(equalTo: detail.bottomAnchor, constant: 7), spinner.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -14), spinner.centerYAnchor.constraint(equalTo: centerYAnchor), spinner.widthAnchor.constraint(equalToConstant: 16), spinner.heightAnchor.constraint(equalToConstant: 16), result.trailingAnchor.constraint(lessThanOrEqualTo: spinner.leadingAnchor, constant: -8)])
        setAccessibilityLabel(name.stringValue)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    func update(selected: Bool, status: PDFCompressionSession.Status, originalBytes: Int) {
        state = selected ? .on : .off
        layer?.borderColor = (selected ? NSColor.controlAccentColor : NSColor.separatorColor).cgColor
        layer?.backgroundColor = (selected ? NSColor.controlAccentColor.withAlphaComponent(0.08) : NSColor.controlBackgroundColor).cgColor
        detail.stringValue = "原始 " + ByteCountFormatter.string(fromByteCount: Int64(originalBytes), countStyle: .file)
        result.stringValue = status.label; result.textColor = status.isFailure ? .systemRed : .labelColor
        if case .processing = status { spinner.isHidden = false; spinner.startAnimation(nil) }
        else { spinner.isHidden = true; spinner.stopAnimation(nil) }
        setAccessibilityValue((selected ? "已选择；" : "") + status.label)
    }
    override func hitTest(_ point: NSPoint) -> NSView? { super.hitTest(point) == nil ? nil : self }
}
