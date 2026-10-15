import AppKit
import PDFKit
import PaperCore
import UniformTypeIdentifiers
import Darwin

enum ImageImportPaper: String, CaseIterable {
    case a4, letter, pixels
    var title: String {
        switch self { case .a4: return "A4"; case .letter: return "Letter"; case .pixels: return "图片像素尺寸" }
    }
    var size: CGSize? {
        switch self {
        case .a4: return CGSize(width: 595.276, height: 841.89)
        case .letter: return CGSize(width: 612, height: 792)
        case .pixels: return nil
        }
    }
}

enum ImageImportOrientation: String, CaseIterable {
    case portrait, landscape
    var title: String { self == .portrait ? "纵向" : "横向" }
}

struct ImageImportOutput {
    let data: Data
    let pageCount: Int
    let suggestedName: String
}

enum ImageImportControllerFailure: Error, Equatable {
    case invalidInput, unsupportedFormat, tooManyFiles, invalidMargin, busy, cancelled, staleResult, invalidOutput
}

/// Pins regular source files without following links, stages only bounded bytes in the
/// existing job's owned directory, and enforces the advertised actual encoding subset.
private enum ImageImportPreparation {
    static func stage(_ files: [URL], in directory: URL, cancellation: LocalToolCancellation) throws -> [URL] {
        var remaining = 512 * 1024 * 1024
        return try files.enumerated().map { index, input in
            guard !cancellation.isCancelled else { throw ImageImportControllerFailure.cancelled }
            let descriptor = Darwin.open(input.path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK)
            guard descriptor >= 0 else { throw ImportFailure.invalidInput }
            let source = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
            defer { try? source.close() }
            var before = stat()
            guard fstat(descriptor, &before) == 0, (before.st_mode & S_IFMT) == S_IFREG, before.st_size > 0 else {
                throw ImportFailure.invalidInput
            }
            let budget = min(128 * 1024 * 1024, remaining)
            guard before.st_size <= budget else { throw ImportFailure.tooLarge }
            let header = try source.read(upToCount: min(16, budget)) ?? Data()
            guard header.count <= budget else { throw ImportFailure.tooLarge }
            let bytes = [UInt8](header)
            let png = bytes.starts(with: [137, 80, 78, 71, 13, 10, 26, 10])
            let jpeg = bytes.starts(with: [255, 216, 255])
            let tiff = [[73, 73, 42, 0], [77, 77, 0, 42], [73, 73, 43, 0], [77, 77, 0, 43]].contains { bytes.starts(with: $0.map(UInt8.init)) }
            guard png || jpeg || tiff else { throw ImageImportControllerFailure.unsupportedFormat }
            let destination = directory.appendingPathComponent("image-\(index).\(input.pathExtension.lowercased())")
            let destinationFD = Darwin.open(destination.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
            guard destinationFD >= 0 else { throw ImportFailure.writeFailed }
            let target = FileHandle(fileDescriptor: destinationFD, closeOnDealloc: true)
            defer { try? target.close() }
            try target.write(contentsOf: header)
            var copied = header.count
            while true {
                guard !cancellation.isCancelled else { throw ImageImportControllerFailure.cancelled }
                let chunk = try source.read(upToCount: min(64 * 1024, budget - copied + 1)) ?? Data()
                guard chunk.count <= budget - copied else { throw ImportFailure.tooLarge }
                if chunk.isEmpty { break }
                try target.write(contentsOf: chunk); copied += chunk.count
            }
            var after = stat()
            guard lstat(input.path, &after) == 0, before.st_ino == after.st_ino, before.st_dev == after.st_dev,
                  copied == before.st_size, before.st_size == after.st_size,
                  before.st_mtimespec.tv_sec == after.st_mtimespec.tv_sec,
                  before.st_mtimespec.tv_nsec == after.st_mtimespec.tv_nsec else {
                throw ImageImportControllerFailure.staleResult
            }
            remaining -= copied
            return destination
        }
    }
}

/// Standalone native sheet candidate. Produces a new document; never mutates a workspace
/// or writes to a caller-selected destination. The parent owns new-tab admission/save UI.
@MainActor
final class ImageImportController: NSViewController, NSTableViewDataSource, NSTableViewDelegate, NSWindowDelegate {
    var onImport: ((ImageImportOutput) -> Void)?
    var onStatus: ((String) -> Void)?
    var onFailure: ((Error) -> Void)?
    private(set) var inputs: [URL] = []
    private(set) var paper: ImageImportPaper = .a4
    private(set) var orientation: ImageImportOrientation = .portrait
    private(set) var layout = ImageLayout()
    private(set) var isRunning = false

    let paperChooser = NSPopUpButton(frame: .zero, pullsDown: false)
    let orientationChooser = NSPopUpButton(frame: .zero, pullsDown: false)
    let marginField = NSTextField(string: "24")
    let fileTable = NSTableView()
    let convertButton = NSButton(title: "创建新 PDF", target: nil, action: nil)
    let formatNotice = NSTextField(wrappingLabelWithString: "")
    private let upButton = NSButton(title: "上移", target: nil, action: nil)
    private let downButton = NSButton(title: "下移", target: nil, action: nil)
    private let removeButton = NSButton(title: "移除", target: nil, action: nil)
    private let cancelButton = NSButton(title: "取消", target: nil, action: nil)
    private let statusLabel = NSTextField(wrappingLabelWithString: "")
    private let progress = NSProgressIndicator()
    private var generation = UUID()
    private var cancellation: LocalToolCancellation?
    private var conversionTask: Task<Void, Never>?
    private var configurationWindow: NSWindow?
    private var filePanel: NSOpenPanel?

    private struct ConversionRequest {
        let files: [URL]
        let layout: ImageLayout
        let generation: UUID
        let cancellation: LocalToolCancellation
        let activity: UUID
    }

    init() { super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func replaceInputs(_ files: [URL]) throws {
        guard files.count <= 500 else { throw ImageImportControllerFailure.tooManyFiles }
        var next: [URL] = []
        for file in files {
            guard file.isFileURL else { throw ImageImportControllerFailure.invalidInput }
            guard ["png", "jpg", "jpeg", "tif", "tiff"].contains(file.pathExtension.lowercased()) else {
                throw ImageImportControllerFailure.unsupportedFormat
            }
            let normalized = file.standardizedFileURL
            if !next.contains(normalized) { next.append(normalized) }
        }
        invalidate()
        inputs = next
        refreshControls()
    }

    func setLayout(paper: ImageImportPaper, orientation: ImageImportOrientation, marginText: String) throws {
        let next: ImageLayout
        if var size = paper.size {
            guard let margin = Double(marginText.trimmingCharacters(in: .whitespacesAndNewlines)),
                  margin.isFinite, margin >= 0, margin * 2 < min(size.width, size.height) else {
                throw ImageImportControllerFailure.invalidMargin
            }
            if orientation == .landscape { size = CGSize(width: size.height, height: size.width) }
            next = ImageLayout(size: size, margin: margin)
        } else { next = ImageLayout(size: nil, margin: 0) }
        if self.paper != paper || self.orientation != orientation || layout.size != next.size || layout.margin != next.margin {
            invalidate()
        }
        self.paper = paper; self.orientation = orientation; layout = next
        refreshControls()
    }

    @discardableResult func moveInput(at index: Int, offset: Int) -> Bool {
        guard inputs.indices.contains(index), offset == -1 || offset == 1,
              inputs.indices.contains(index + offset) else { return false }
        invalidate(); inputs.swapAt(index, index + offset)
        refreshControls(select: index + offset)
        return true
    }

    @discardableResult func removeInput(at index: Int) -> Bool {
        guard inputs.indices.contains(index) else { return false }
        invalidate(); inputs.remove(at: index)
        refreshControls(select: min(index, inputs.count - 1))
        return true
    }

    /// The button action reserves its flag synchronously, so cancellation also covers
    /// queued work that has not entered the asynchronous conversion method yet.
    func cancel() { cancellation?.cancel(); conversionTask?.cancel() }

    /// Call when the parent discards/replaces this import session. Old work cannot publish.
    func invalidate() { generation = UUID(); cancel() }

    func convert() async throws -> ImageImportOutput {
        try await performConversion(reserveConversion())
    }

    private func reserveConversion() throws -> ConversionRequest {
        guard !isRunning else { throw ImageImportControllerFailure.busy }
        guard !inputs.isEmpty else { throw ImageImportControllerFailure.invalidInput }
        let flag = LocalToolCancellation()
        cancellation = flag; isRunning = true
        let request = ConversionRequest(files: inputs, layout: layout, generation: generation,
                                        cancellation: flag, activity: LocalToolActivity.register(flag))
        refreshControls()
        return request
    }

    private func validate(_ request: ConversionRequest) throws {
        guard generation == request.generation else { throw ImageImportControllerFailure.staleResult }
        guard !request.cancellation.isCancelled, !Task.isCancelled else { throw ImageImportControllerFailure.cancelled }
    }

    private func performConversion(_ request: ConversionRequest) async throws -> ImageImportOutput {
        let files = request.files, options = request.layout, flag = request.cancellation
        defer {
            isRunning = false; cancellation = nil
            LocalToolActivity.finish(request.activity)
            refreshControls()
        }
        do {
            try validate(request) // A discarded pending click must not emit even a status callback.
            onStatus?("正在转换图片 · 不覆盖来源文件或当前文档")
            try validate(request) // Status handlers may synchronously cancel/discard the session.
            let result = try await withTaskCancellationHandler {
                try await LocalToolJob.run(data: Data(), cancellation: flag) { _, output, cancellation in
                    let staged = try ImageImportPreparation.stage(files, in: output.deletingLastPathComponent(), cancellation: cancellation)
                    let count = try ImagePDFImport.convert(staged, to: output, layout: options, cancelled: { cancellation.isCancelled })
                    return String(count)
                }
            } onCancel: { flag.cancel() }
            try validate(request)
            guard let expected = Int(result.message), expected > 0,
                  let document = PDFDocument(data: result.data), document.pageCount == expected else {
                throw ImageImportControllerFailure.invalidOutput
            }
            let base = files.count == 1 ? files[0].deletingPathExtension().lastPathComponent : "图片合并"
            let output = ImageImportOutput(data: result.data, pageCount: expected, suggestedName: base + ".pdf")
            try validate(request) // PDFKit validation can take time; cancellation wins until publication.
            closeConfigurationSheet()
            try validate(request)
            onImport?(output)
            return output
        } catch {
            if generation != request.generation { throw ImageImportControllerFailure.staleResult }
            if flag.isCancelled || Task.isCancelled { throw ImageImportControllerFailure.cancelled }
            throw error
        }
    }

    /// Native file picker followed by this controller's native configuration sheet.
    /// Keep the controller strongly retained in the parent while either sheet/job is active.
    func present(from window: NSWindow) {
        guard !isRunning, filePanel == nil, configurationWindow == nil, window.attachedSheet == nil else {
            onFailure?(ImageImportControllerFailure.busy); return
        }
        invalidate()
        let version = generation
        let panel = NSOpenPanel()
        panel.title = "选择图片创建新 PDF"
        panel.message = "仅支持静态 PNG、JPEG 与 TIFF（全部帧）。可多选，下一步可调整顺序；不修改来源图片。"
        panel.allowedContentTypes = [.png, .jpeg, .tiff]
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        panel.resolvesAliases = false
        filePanel = panel
        panel.beginSheetModal(for: window) { [weak self, weak window] response in
            guard let self else { return }
            self.filePanel = nil
            guard response == .OK, self.generation == version, let window else { return }
            do {
                try self.replaceInputs(panel.urls)
                guard !self.inputs.isEmpty else { return }
                self.showConfigurationSheet(from: window)
            } catch { self.onFailure?(error) }
        }
    }

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 620, height: 500))
        view = root
        let title = NSTextField(labelWithString: "图片转 PDF · 本地候选功能")
        title.font = .systemFont(ofSize: 17, weight: .semibold)
        formatNotice.font = .systemFont(ofSize: 11)
        formatNotice.textColor = .secondaryLabelColor
        paperChooser.addItems(withTitles: ImageImportPaper.allCases.map(\.title))
        orientationChooser.addItems(withTitles: ImageImportOrientation.allCases.map(\.title))
        bind(paperChooser, label: "纸张大小", action: #selector(changeLayout(_:)))
        bind(orientationChooser, label: "页面方向", action: #selector(changeLayout(_:)))
        bind(marginField, label: "页边距，单位点", action: #selector(changeLayout(_:)))
        marginField.widthAnchor.constraint(equalToConstant: 65).isActive = true
        let options = NSStackView(views: [NSTextField(labelWithString: "纸型"), paperChooser,
            NSTextField(labelWithString: "方向"), orientationChooser,
            NSTextField(labelWithString: "边距（pt）"), marginField])
        options.orientation = .horizontal; options.spacing = 8
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("filename"))
        column.title = "转换顺序 / 来源图片"; column.width = 540
        fileTable.addTableColumn(column)
        fileTable.delegate = self; fileTable.dataSource = self
        fileTable.allowsMultipleSelection = false
        fileTable.rowHeight = 26
        fileTable.setAccessibilityLabel("图片顺序，每个文件按列表次序转换")
        let scroll = NSScrollView(); scroll.hasVerticalScroller = true
        scroll.borderType = .bezelBorder; scroll.documentView = fileTable
        scroll.heightAnchor.constraint(equalToConstant: 230).isActive = true
        let actions = NSStackView(views: [upButton, downButton, removeButton])
        actions.orientation = .horizontal; actions.spacing = 8
        bind(upButton, label: "上移图片", action: #selector(moveSelectedImageUp(_:)))
        bind(downButton, label: "下移图片", action: #selector(moveSelectedImageDown(_:)))
        bind(removeButton, label: "移除图片，不删除来源文件", action: #selector(removeSelected(_:)))
        bind(convertButton, label: "创建新 PDF，不覆盖当前文档", action: #selector(startConversion(_:)))
        bind(cancelButton, label: "取消图片导入", action: #selector(cancelSheet(_:)))
        convertButton.keyEquivalent = "\r"; cancelButton.keyEquivalent = "\u{1b}"
        progress.style = .spinning; progress.controlSize = .small; progress.isDisplayedWhenStopped = false
        statusLabel.font = .systemFont(ofSize: 11)
        statusLabel.setAccessibilityLabel("图片导入状态")
        let footer = NSStackView(views: [progress, cancelButton, convertButton])
        footer.orientation = .horizontal; footer.spacing = 10
        let stack = NSStackView(views: [title, formatNotice, options, scroll, actions, statusLabel, footer])
        stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -20),
            stack.topAnchor.constraint(equalTo: root.topAnchor, constant: 20),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: root.bottomAnchor, constant: -16),
            scroll.widthAnchor.constraint(equalTo: stack.widthAnchor),
            formatNotice.widthAnchor.constraint(equalTo: stack.widthAnchor),
            statusLabel.widthAnchor.constraint(equalTo: stack.widthAnchor)
        ])
        refreshControls()
    }

    private func bind(_ control: NSControl, label: String, action: Selector) {
        control.target = self; control.action = action
        control.setAccessibilityLabel(label); control.toolTip = label
        if let button = control as? NSButton { button.bezelStyle = .rounded }
    }

    private func refreshControls(select index: Int? = nil) {
        guard isViewLoaded else { return }
        paperChooser.selectItem(at: ImageImportPaper.allCases.firstIndex(of: paper) ?? 0)
        orientationChooser.selectItem(at: ImageImportOrientation.allCases.firstIndex(of: orientation) ?? 0)
        marginField.stringValue = String(layout.margin)
        paperChooser.isEnabled = !isRunning
        orientationChooser.isEnabled = !isRunning && paper != .pixels
        marginField.isEnabled = !isRunning && paper != .pixels
        convertButton.isEnabled = !isRunning && !inputs.isEmpty
        fileTable.isEnabled = !isRunning
        fileTable.reloadData()
        if let index, inputs.indices.contains(index) { fileTable.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false) }
        updateOrderButtons()
        formatNotice.stringValue = "仅静态 PNG/JPEG 与 TIFF；TIFF 全帧逐页转换，总计最多 500 页。不支持 GIF、HEIC、SVG；无 OCR。\n保持比例居中；图片像素尺寸：1 像素＝1 PDF 点（不猜测打印 DPI），方向与边距不适用。"
    }

    private func updateOrderButtons() {
        let index = fileTable.selectedRow
        upButton.isEnabled = !isRunning && index > 0
        downButton.isEnabled = !isRunning && index >= 0 && index + 1 < inputs.count
        removeButton.isEnabled = !isRunning && inputs.indices.contains(index)
    }

    func numberOfRows(in tableView: NSTableView) -> Int { inputs.count }
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard inputs.indices.contains(row) else { return nil }
        let text = NSTextField(labelWithString: "\(row + 1).  \(inputs[row].lastPathComponent)")
        text.lineBreakMode = .byTruncatingMiddle
        text.toolTip = inputs[row].path
        return text
    }
    func tableViewSelectionDidChange(_ notification: Notification) { updateOrderButtons() }

    private func showConfigurationSheet(from parent: NSWindow) {
        _ = view
        statusLabel.stringValue = "按列表顺序创建独立 PDF；来源文件和当前文档保持不变。"
        let sheet = NSWindow(contentViewController: self)
        sheet.title = "图片转 PDF"; sheet.delegate = self
        sheet.setContentSize(NSSize(width: 620, height: 500))
        configurationWindow = sheet
        parent.beginSheet(sheet) { [weak self] _ in self?.configurationWindow = nil }
    }

    private func closeConfigurationSheet() {
        guard let sheet = configurationWindow else { return }
        sheet.sheetParent?.endSheet(sheet)
        sheet.orderOut(nil)
        configurationWindow = nil
    }

    @objc private func changeLayout(_ sender: NSControl) {
        do { try readLayoutControls(); statusLabel.stringValue = "已更新纸型和边距；来源图片保持不变。" }
        catch { statusLabel.stringValue = Self.message(for: error) }
    }

    private func readLayoutControls() throws {
        guard ImageImportPaper.allCases.indices.contains(paperChooser.indexOfSelectedItem),
              ImageImportOrientation.allCases.indices.contains(orientationChooser.indexOfSelectedItem) else {
            throw ImageImportControllerFailure.invalidMargin
        }
        try setLayout(paper: ImageImportPaper.allCases[paperChooser.indexOfSelectedItem],
                      orientation: ImageImportOrientation.allCases[orientationChooser.indexOfSelectedItem],
                      marginText: marginField.stringValue)
    }

    @objc private func moveSelectedImageUp(_ sender: NSButton) { _ = moveInput(at: fileTable.selectedRow, offset: -1) }
    @objc private func moveSelectedImageDown(_ sender: NSButton) { _ = moveInput(at: fileTable.selectedRow, offset: 1) }
    @objc private func removeSelected(_ sender: NSButton) { _ = removeInput(at: fileTable.selectedRow) }

    @objc private func startConversion(_ sender: NSButton) {
        guard !isRunning else { return }
        let request: ConversionRequest
        do { try readLayoutControls(); request = try reserveConversion() }
        catch { statusLabel.stringValue = Self.message(for: error); return }
        progress.startAnimation(nil)
        statusLabel.stringValue = "正在转换图片…取消将丢弃全部结果。"
        // Retain the owner until the reserved activity is finished, including queued cancellation.
        conversionTask = Task { [self] in
            defer { progress.stopAnimation(nil); conversionTask = nil }
            do { _ = try await performConversion(request) }
            catch {
                switch error as? ImageImportControllerFailure {
                case .cancelled, .staleResult: break // Dismissed/obsolete sessions have no terminal callbacks.
                default: statusLabel.stringValue = Self.message(for: error); onFailure?(error)
                }
            }
        }
    }

    @objc private func cancelSheet(_ sender: NSButton) {
        cancel()
        closeConfigurationSheet()
    }
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        cancel(); closeConfigurationSheet(); return false
    }

    static func message(for error: Error) -> String {
        switch error as? ImageImportControllerFailure {
        case .invalidInput: return "请选择本机 PNG、JPEG 或 TIFF 图片。"
        case .unsupportedFormat: return "该格式不在本候选范围：仅 PNG、JPEG、TIFF；不支持 GIF、HEIC、SVG。"
        case .tooManyFiles: return "最多选择 500 个文件；TIFF 多帧合计也不得超过 500 页。"
        case .invalidMargin: return "边距须为非负有限点数，且小于纸张短边的一半。"
        case .busy: return "正在选择或转换图片，请先完成或取消当前任务。"
        case .cancelled: return "已取消图片转换；未发布 PDF，来源文件和当前文档未修改。"
        case .staleResult: return "导入配置已改变，旧结果已丢弃；请重新转换。"
        case .invalidOutput: return "转换结果未通过页数检查，未创建新文档。"
        case nil: break
        }
        switch error as? ImportFailure {
        case .tooLarge: return "图片字节、像素或页数超过安全上限，未发布结果。"
        case .invalidInput, .decodeFailed: return "图片不存在、无法解码或含不支持的多帧结构，未发布结果。"
        case .invalidLayout: return "纸型或页边距无效，未发布结果。"
        case .cancelled: return "已取消图片转换，未发布结果。"
        default: return "图片转换失败，未发布结果：\(error.localizedDescription)"
        }
    }
}
