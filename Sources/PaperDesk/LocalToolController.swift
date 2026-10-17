import AppKit
import PaperCore

@MainActor enum LocalToolActivity {
    private static var jobs: [UUID: LocalToolCancellation] = [:]
    static var hasRunningJobs: Bool { !jobs.isEmpty }
    static func register(_ flag: LocalToolCancellation) -> UUID { let id = UUID(); jobs[id] = flag; return id }
    static func finish(_ id: UUID) { jobs.removeValue(forKey: id) }
    static func cancelAndWait() async {
        while !jobs.isEmpty {
            jobs.values.forEach { $0.cancel() }
            try? await Task.sleep(nanoseconds: 20_000_000)
        }
    }
}

enum LocalOrganizeOperation {
    case numbers(NumberOptions), compress, ocr
    var title: String {
        switch self { case .numbers: return "添加页码"; case .compress: return "无损压缩"; case .ocr: return "文字识别" }
    }
}

@MainActor extension WorkbenchController {
    func cancelLocalTool() { localToolCancellation?.cancel() }

    func applyLocalTool(_ operation: LocalOrganizeOperation, qpdf: URL) async throws {
        guard !isLocalToolRunning else { throw OrganizeError.unavailable }
        let input = try workspace.engineInput()
        let cancellation = LocalToolCancellation(); localToolCancellation = cancellation
        let activity = LocalToolActivity.register(cancellation)
        defer { localToolCancellation = nil; LocalToolActivity.finish(activity) }
        let result = try await LocalToolJob.run(data: input.data, cancellation: cancellation) { source, output, flag in
            let engine = OrganizeEngine(qpdf: qpdf, scratch: source.deletingLastPathComponent())
            let cancelled = { flag.isCancelled }
            switch operation {
            case .numbers(let options):
                try engine.number(source, output: output, options: options, cancelled: cancelled)
                return "页码已添加 · 可撤销 · 请存储副本"
            case .compress:
                let r = try engine.compress(source, output: output, cancelled: cancelled)
                return r.reduced ? "无损优化：\(r.before) → \(r.after) 字节 · 请存储副本" : "未发现可无损减小的空间"
            case .ocr:
                let pages = try engine.ocr(source, output: output, cancelled: cancelled)
                return "OCR：处理 \(pages.filter { !$0.skipped }.count) 页，跳过 \(pages.filter(\.skipped).count) 页 · 通用文字提取兼容性未通过"
            }
        }
        guard !cancellation.isCancelled else { throw OrganizeError.cancelled }
        // Reject results if the user edited/replaced the document while work was running.
        try workspace.acceptEngineResult(result.data, for: input)
        refreshDocument(); setInteractionStatus(result.message)
    }

    func presentLocalTool(_ id: String) {
        guard let window = view.window, !isLocalToolRunning else { return }
        guard let qpdf = Bundle.main.resourceURL?.appendingPathComponent("Engines/qpdf/bin/qpdf"),
              FileManager.default.isExecutableFile(atPath: qpdf.path) else {
            setInteractionStatus("缺少随 App 打包的本地处理引擎，未修改文档"); return
        }
        if id == "compress" { presentCompressionOptions(qpdf: qpdf, window: window); return }
        let alert = NSAlert()
        alert.addButton(withTitle: "开始"); alert.addButton(withTitle: "取消")
        var operation = LocalOrganizeOperation.compress
        let start = NSTextField(string: "1"), prefix = NSTextField(string: ""), suffix = NSTextField(string: "")
        let top = NSButton(checkboxWithTitle: "放在页面顶部（默认底部）", target: nil, action: nil)
        switch id {
        case "page-numbers":
            alert.messageText = "添加页码"
            alert.informativeText = "所有页面居中添加页码；不替换既有页码。仅支持未旋转、未裁剪的页面。"
            let fields = NSStackView(); fields.orientation = .vertical; fields.alignment = .leading; fields.spacing = 10
            for (title, field) in [("起始数字", start), ("前缀", prefix), ("后缀", suffix)] {
                field.widthAnchor.constraint(equalToConstant: 210).isActive = true
                let row = NSStackView(views: [NSTextField(labelWithString: title), field]); row.spacing = 12; fields.addArrangedSubview(row)
            }
            fields.addArrangedSubview(top); fields.frame = NSRect(x: 0, y: 0, width: 310, height: 145); alert.accessoryView = fields
        case "ocr":
            operation = .ocr; alert.messageText = "文字识别 · 简体中文 / 英文"
            alert.informativeText = "在本机识别清晰的水平扫描页，跳过已有文字的页面。暂不支持旋转或裁剪页；识别结果支持本机检索，导出后在其他阅读器中的文字提取效果可能不同。请核对识别结果。"
        default: return
        }
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            if id == "page-numbers" {
                guard let number = Int(start.stringValue), number >= 0 else { self.setInteractionStatus("起始数字必须是非负整数"); return }
                var options = NumberOptions(); options.start = number; options.prefix = prefix.stringValue; options.suffix = suffix.stringValue; options.top = top.state == .on
                operation = .numbers(options)
            }
            self.runLocalToolSheet(operation, qpdf: qpdf, window: window)
        }
    }

    private func runLocalToolSheet(_ operation: LocalOrganizeOperation, qpdf: URL, window: NSWindow) {
        let progress = NSAlert(); progress.messageText = "\(operation.title)…"
        progress.informativeText = "在本机处理当前编辑快照，不覆盖原文件。取消会丢弃本次结果。"
        progress.addButton(withTitle: "取消处理")
        let indicator = NSProgressIndicator(frame: NSRect(x: 0, y: 0, width: 320, height: 16))
        indicator.isIndeterminate = true; indicator.style = .bar; indicator.startAnimation(nil); progress.accessoryView = indicator
        progress.beginSheetModal(for: window) { [weak self] response in
            if response == .alertFirstButtonReturn { self?.cancelLocalTool() }
        }
        Task { [weak self] in
            guard let self else { return }
            do { try await self.applyLocalTool(operation, qpdf: qpdf) }
            catch { self.setInteractionStatus(Self.localToolError(error)) }
            indicator.stopAnimation(nil)
            if progress.window.sheetParent != nil { window.endSheet(progress.window, returnCode: .stop) }
        }
    }

    static func localToolError(_ error: Error) -> String {
        if let error = error as? PDFWorkspaceError { return WorkbenchMessage.error(error) }
        switch error as? OrganizeError {
        case .cancelled: return "已取消处理 · 文档未修改"
        case .unsupportedGeometry: return "暂不支持旋转、裁剪或非零原点页面 · 文档未修改"
        case .encrypted: return "此处理工具暂不支持加密文档"
        case .signed: return "检测到签名字段，拒绝处理以免破坏签名"
        case .invalidInput: return "输入或页码设置不受支持 · 文档未修改"
        default: return "处理未成功，文档未修改：\(error.localizedDescription)"
        }
    }
}
