import AppKit
import PDFKit
import PaperCore
import UniformTypeIdentifiers

@MainActor extension WorkbenchController {
    func selectContentTool(_ id: String) {
        guard !isLocalToolRunning else { return }
        guard let engine = contentEngine() else {
            setInteractionStatus("未找到内容编辑组件（Python 3.12 / PyMuPDF）。文档未修改。")
            return
        }
        resetDocumentInteractionState(); contentToolID = id; pdfView.contentEditingEnabled = true
        let action = id == "content-text" ? "点击原文文字" : id == "content-image" ? "点击原图" : "拖出目标区域"
        setInteractionStatus("\(action) · 复杂字体或共享对象可能不支持 · Esc 退出")
        pdfView.onContentSelection = { [weak self] start, end, page in
            self?.prepareContentAction(id, start: start, end: end, page: page, engine: engine)
        }
    }

    private func contentEngine() -> ContentEditEngine? {
        guard FileManager.default.isExecutableFile(atPath: ContentEditEngine.localPython.path),
              let script = Bundle.main.resourceURL?.appendingPathComponent("Engines/content-edit/engine.py"),
              FileManager.default.fileExists(atPath: script.path) else { return nil }
        return ContentEditEngine(script: script)
    }

    func applyContentEdit(_ request: ContentEditRequest, input: PDFEngineInput, engine: ContentEditEngine) async throws {
        guard !isLocalToolRunning else { throw OrganizeError.unavailable }
        try workspace.validateEngineInput(input)
        let cancellation = LocalToolCancellation(); localToolCancellation = cancellation
        let activity = LocalToolActivity.register(cancellation)
        defer { localToolCancellation = nil; LocalToolActivity.finish(activity) }
        let result = try await LocalToolJob.run(data: input.data, cancellation: cancellation) { source, output, flag in
            for raw in input.safetySourceDatas { try engine.validateAdmission(raw, in: source.deletingLastPathComponent(), cancelled: { flag.isCancelled }) }
            return try engine.apply(source, request: request, output: output, cancelled: { flag.isCancelled })
        }
        guard !cancellation.isCancelled else { throw OrganizeError.cancelled }
        try workspace.acceptEngineResult(result.data, for: input)
        refreshDocument(preservePage: request.page)
        setInteractionStatus(result.message + " · 可撤销 · 请存储副本")
    }

    private func prepareContentAction(_ id: String, start: CGPoint, end: CGPoint, page: Int, engine: ContentEditEngine) {
        guard !isLocalToolRunning, let pdfPage = pdfView.document?.page(at: page), let window = view.window else { return }
        let media = pdfPage.bounds(for: .mediaBox)
        let point = CGPoint(x: start.x - media.minX, y: media.maxY - start.y)
        let other = CGPoint(x: end.x - media.minX, y: media.maxY - end.y)
        let box = [min(point.x, other.x), min(point.y, other.y), max(point.x, other.x), max(point.y, other.y)].map(Double.init)
        let input: PDFEngineInput
        do { input = try workspace.engineInput() } catch { setInteractionStatus(Self.contentError(error)); return }
        let hasArea = box[2] - box[0] >= 8 && box[3] - box[1] >= 8
        if (id == "content-link" && hasArea) || id == "content-redact" {
            guard box[2] - box[0] >= 8, box[3] - box[1] >= 8 else { setInteractionStatus("请拖出至少 8×8 点的目标区域"); return }
            var request = ContentEditRequest(operation: id == "content-link" ? "insert-link" : "redact", page: page); request.bbox = box
            let alert = NSAlert(); alert.messageText = id == "content-link" ? "添加 PDF 链接" : "真正移除区域内容"
            let uri = NSTextField(string: "https://"); uri.frame.size = NSSize(width: 360, height: 24)
            if id == "content-link" { alert.informativeText = "写入 PDF 链接对象。链接不会在编辑时自动打开。"; alert.accessoryView = uri }
            else { alert.informativeText = "将实际删除区域内文字、相交图像和矢量对象，而非盖黑块。相交对象可能整体移除；复杂结构拒绝处理。仅修改当前快照，原文件保留；应用内撤销仍会保留历史，存储副本才得到重写结果。" }
            alert.addButton(withTitle: id == "content-link" ? "添加" : "移除内容"); alert.addButton(withTitle: "取消")
            alert.beginSheetModal(for: window) { [weak self] response in
                guard let self, response == .alertFirstButtonReturn else { return }
                if id == "content-link" {
                    guard let url = URL(string: uri.stringValue), ["https", "http", "mailto"].contains(url.scheme?.lowercased() ?? "") else { self.setInteractionStatus("仅支持 http、https、mailto 链接"); return }
                    request.uri = uri.stringValue
                }
                self.runContentEditSheet(request, input: input, engine: engine, window: window)
            }
            return
        }
        let cancellation = LocalToolCancellation(); localToolCancellation = cancellation
        let activity = LocalToolActivity.register(cancellation)
        setInteractionStatus("正在定位页面原文对象…")
        Task { [weak self] in
            guard let self else { LocalToolActivity.finish(activity); return }
            do {
                let result = try await LocalToolJob.run(data: input.data, cancellation: cancellation) { source, output, flag in
                    for raw in input.safetySourceDatas { try engine.validateAdmission(raw, in: source.deletingLastPathComponent(), cancelled: { flag.isCancelled }) }
                    let inspected = try engine.inspect(source, page: page, cancelled: { flag.isCancelled })
                    try FileManager.default.copyItem(at: source, to: output)
                    return String(decoding: try JSONEncoder().encode(inspected), as: UTF8.self)
                }
                self.localToolCancellation = nil; LocalToolActivity.finish(activity)
                guard !cancellation.isCancelled, self.activeMode == .editing, self.contentToolID == id else { return }
                try self.workspace.validateEngineInput(input)
                let inspected = try JSONDecoder().decode(ContentPage.self, from: Data(result.message.utf8))
                func contains(_ bbox: [Double]) -> Bool { bbox.count == 4 && point.x >= bbox[0] && point.x <= bbox[2] && point.y >= bbox[1] && point.y <= bbox[3] }
                if id == "content-text", let span = inspected.spans.first(where: { contains($0.bbox) }) {
                    self.presentTextReplacement(span, page: page, input: input, engine: engine, window: window)
                } else if id == "content-image", let image = inspected.images.first(where: { contains($0.bbox) }) {
                    let panel = NSOpenPanel(); panel.allowedContentTypes = [.png, .jpeg]; panel.message = "选择替换该原图的 PNG / JPEG（共享原图暂不支持）"
                    panel.beginSheetModal(for: window) { response in
                        guard response == .OK, let url = panel.url else { return }
                        var request = ContentEditRequest(operation: "replace-image", page: page); request.id = image.id; request.image_path = url.path
                        self.runContentEditSheet(request, input: input, engine: engine, window: window)
                    }
                } else if id == "content-link", let link = inspected.links.first(where: { contains($0.bbox) }) {
                    let alert = NSAlert(); alert.messageText = "编辑 PDF 链接"
                    alert.informativeText = "修改或删除真实链接对象；不会自动打开链接。"
                    let field = NSTextField(string: link.uri); field.frame.size = NSSize(width: 440, height: 28)
                    field.setAccessibilityLabel("PDF 链接地址"); alert.accessoryView = field
                    alert.addButton(withTitle: "应用"); alert.addButton(withTitle: "删除链接"); alert.addButton(withTitle: "取消")
                    alert.beginSheetModal(for: window) { response in
                        guard response == .alertFirstButtonReturn || response == .alertSecondButtonReturn else { return }
                        var request = ContentEditRequest(operation: response == .alertSecondButtonReturn ? "delete-link" : "update-link", page: page)
                        request.id = link.id; if response == .alertFirstButtonReturn { request.uri = field.stringValue }
                        self.runContentEditSheet(request, input: input, engine: engine, window: window)
                    }
                } else { self.setInteractionStatus("该位置没有可编辑的\(id == "content-text" ? "文本行" : "图像对象")；扫描页需先 OCR") }
            } catch {
                self.localToolCancellation = nil; LocalToolActivity.finish(activity)
                self.setInteractionStatus(Self.contentError(error))
            }
        }
    }

    private func presentTextReplacement(_ span: ContentSpan, page: Int, input: PDFEngineInput, engine: ContentEditEngine, window: NSWindow) {
        let alert = NSAlert(); alert.messageText = "编辑原文文字"
        alert.informativeText = "原文字体：\(span.font)，\(span.size) 点。真实移除原文字并写入新文本；字体可能替代。暂不做跨行段落回流，放不下会拒绝，不压缩或覆盖邻文。"
        let field = NSTextField(string: span.text); field.frame.size = NSSize(width: 440, height: 30)
        field.setAccessibilityLabel("替换原文文字"); alert.accessoryView = field
        alert.addButton(withTitle: "应用"); alert.addButton(withTitle: "取消")
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            var request = ContentEditRequest(operation: "replace-text", page: page); request.id = span.id; request.text = field.stringValue
            self.runContentEditSheet(request, input: input, engine: engine, window: window)
        }
    }

    private func runContentEditSheet(_ request: ContentEditRequest, input: PDFEngineInput, engine: ContentEditEngine, window: NSWindow) {
        let progress = NSAlert(); progress.messageText = "正在修改 PDF 内容…"; progress.informativeText = "仅在本机处理当前快照。取消会丢弃结果，不覆盖原文件。"
        progress.addButton(withTitle: "取消处理")
        progress.beginSheetModal(for: window) { [weak self] response in if response == .alertFirstButtonReturn { self?.cancelLocalTool() } }
        Task { [weak self] in
            guard let self else { return }
            do { try await self.applyContentEdit(request, input: input, engine: engine) }
            catch { self.setInteractionStatus(Self.contentError(error)) }
            if progress.window.sheetParent != nil { window.endSheet(progress.window, returnCode: .stop) }
        }
    }
    private static func contentError(_ error: Error) -> String {
        if let e = error as? PDFWorkspaceError { return WorkbenchMessage.error(e) }
        if case .processFailed(let message) = error as? OrganizeError { return "内容未修改：" + message }
        if case .cancelled = error as? OrganizeError { return "已取消 · 内容未修改" }
        return "内容未修改：\(error.localizedDescription)"
    }
}
