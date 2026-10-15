import AppKit
import PaperCore
import PDFKit

private final class PDFImageExportCancellation: @unchecked Sendable {
    private let lock = NSLock()
    private var value = false
    func cancel() { lock.lock(); value = true; lock.unlock() }
    func isCancelled() -> Bool { lock.lock(); defer { lock.unlock() }; return value }
}

@MainActor
final class PDFImageExportController: NSViewController {
    let pageCount: Int
    let suggestedBaseName: String
    private let documentData: Data
    private let formatChooser = NSPopUpButton()
    private let resolutionChooser = NSPopUpButton()
    private let pageField = NSTextField()
    private let statusLabel = NSTextField(wrappingLabelWithString: "")
    private let progress = NSProgressIndicator()
    private let exportButton = NSButton(title: "选择位置并导出…", target: nil, action: nil)
    private let cancelButton = NSButton(title: "取消", target: nil, action: nil)
    private var cancellation: PDFImageExportCancellation?
    private var sheetWindow: NSWindow?
    private(set) var isRunning = false
    var onFinished: ((PDFImageExportResult) -> Void)?
    var onFailure: ((Error) -> Void)?
    var onDismiss: (() -> Void)?

    init(documentData: Data, pageCount: Int, suggestedBaseName: String) {
        self.documentData = documentData
        self.pageCount = pageCount
        self.suggestedBaseName = suggestedBaseName
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 560, height: 300))
        let title = NSTextField(labelWithString: "PDF 转图片 · 本地导出")
        title.font = .systemFont(ofSize: 18, weight: .semibold)
        let notice = NSTextField(wrappingLabelWithString: "使用系统 PDFKit/CoreGraphics 离线渲染。输出到新文件夹，已有文件夹不会被覆盖。")
        notice.textColor = .secondaryLabelColor; notice.font = .systemFont(ofSize: 11)
        formatChooser.addItems(withTitles: ["PNG", "JPEG"])
        resolutionChooser.addItems(withTitles: ["72 DPI", "144 DPI", "300 DPI"]); resolutionChooser.selectItem(at: 1)
        pageField.stringValue = pageCount == 1 ? "1" : "1-\(pageCount)"
        pageField.placeholderString = "例如 1-3, 5"; pageField.setAccessibilityLabel("导出页码范围")
        let options = NSGridView(views: [
            [NSTextField(labelWithString: "格式"), formatChooser],
            [NSTextField(labelWithString: "分辨率"), resolutionChooser],
            [NSTextField(labelWithString: "页码"), pageField]
        ])
        options.rowSpacing = 10; options.columnSpacing = 12; options.column(at: 0).xPlacement = .trailing
        progress.style = .spinning; progress.controlSize = .small; progress.isDisplayedWhenStopped = false
        statusLabel.font = .systemFont(ofSize: 11); statusLabel.textColor = .secondaryLabelColor
        statusLabel.stringValue = "共 \(pageCount) 页；页码按文档顺序导出。"
        exportButton.target = self; exportButton.action = #selector(chooseAndExport)
        exportButton.keyEquivalent = "\r"; exportButton.setAccessibilityLabel("选择新文件夹并导出图片")
        cancelButton.target = self; cancelButton.action = #selector(cancel); cancelButton.keyEquivalent = "\u{1b}"
        let footer = NSStackView(views: [progress, NSView(), cancelButton, exportButton])
        footer.orientation = .horizontal; footer.spacing = 10
        let stack = NSStackView(views: [title, notice, options, statusLabel, footer])
        stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 16
        stack.translatesAutoresizingMaskIntoConstraints = false; root.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 22),
            stack.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -22),
            stack.topAnchor.constraint(equalTo: root.topAnchor, constant: 22),
            footer.widthAnchor.constraint(equalTo: stack.widthAnchor), notice.widthAnchor.constraint(equalTo: stack.widthAnchor),
            pageField.widthAnchor.constraint(equalToConstant: 260)
        ])
        view = root
    }

    func present(from window: NSWindow) {
        _ = view
        let sheet = NSWindow(contentViewController: self); sheet.title = "PDF 转图片"; sheet.styleMask = [.titled]
        sheet.setContentSize(view.frame.size); sheetWindow = sheet; window.beginSheet(sheet)
    }

    @discardableResult
    func startExport(request: PDFImageExportRequest, destination: URL) -> Bool {
        guard !isRunning else { return false }
        isRunning = true
        let token = PDFImageExportCancellation(); cancellation = token
        progress.startAnimation(nil); exportButton.isEnabled = false
        statusLabel.stringValue = "正在离线渲染并验证输出…"
        let data = documentData
        Task { [weak self] in
            let outcome = await Task.detached(priority: .userInitiated) { () -> Result<PDFImageExportResult, PDFImageExportFailure> in
                do { return .success(try PDFImageExport.export(data: data, to: destination, request: request, cancelled: token.isCancelled)) }
                catch let failure as PDFImageExportFailure { return .failure(failure) }
                catch { return .failure(.writeFailed) }
            }.value
            guard let self, self.cancellation === token else { return }
            self.isRunning = false; self.cancellation = nil
            self.progress.stopAnimation(nil); self.exportButton.isEnabled = true
            switch outcome {
            case .success(let result):
                self.statusLabel.stringValue = "已导出 \(result.files.count) 张图片：\(result.directory.lastPathComponent)"
                self.onFinished?(result)
            case .failure(let error):
                self.statusLabel.stringValue = error.localizedDescription; self.onFailure?(error)
            }
        }
        return true
    }

    @objc private func chooseAndExport() {
        do {
            let pages = try PageRange.parse(pageField.stringValue, count: pageCount)
            let format: PDFImageExportFormat = formatChooser.indexOfSelectedItem == 0 ? .png : .jpeg
            let dpis = [72.0, 144.0, 300.0]
            let dpi = dpis[max(0, min(resolutionChooser.indexOfSelectedItem, dpis.count - 1))]
            let panel = NSSavePanel(); panel.title = "选择新的图片输出文件夹"; panel.prompt = "导出"
            panel.canCreateDirectories = true; panel.nameFieldStringValue = suggestedBaseName + "-images"
            guard panel.runModal() == .OK, let destination = panel.url else { return }
            _ = startExport(request: PDFImageExportRequest(format: format, dpi: dpi, pages: pages), destination: destination)
        } catch { statusLabel.stringValue = error.localizedDescription; onFailure?(error) }
    }

    @objc private func cancel() {
        if isRunning { cancellation?.cancel(); statusLabel.stringValue = "正在取消…"; return }
        if let sheetWindow, let parent = sheetWindow.sheetParent { parent.endSheet(sheetWindow) }
        onDismiss?()
    }
}
