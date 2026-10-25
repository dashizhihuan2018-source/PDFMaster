import AppKit
import PDFKit
import PaperCore
import UniformTypeIdentifiers

@MainActor extension WorkbenchController {
    func selectContentTool(_ id: String) {
        guard !isLocalToolRunning, pdfView.finishInlineTextEditing() else { return }
        let engine = contentEngine()
        guard engine != nil || id == "content-text" else {
            setInteractionStatus("未找到原生内容编辑组件。文档未修改。")
            return
        }
        resetDocumentInteractionState(); contentToolID = id; pdfView.contentEditingEnabled = true
        let action = id == "content-text" ? "空白处输入；已写入文字单击拖动、双击修改；原文点击就地修改" : id == "content-image" ? "点击原图" : "拖出目标区域"
        setInteractionStatus("\(action) · 复杂字体或共享对象可能不支持 · Esc 退出")
        pdfView.onContentSelection = { [weak self] start, end, page in
            guard let self else { return }
            if id == "content-text", let pdfPage = self.pdfView.document?.page(at: page) {
                let character = pdfPage.characterIndex(at: start)
                let hitsText = character != NSNotFound && character >= 0 && character < pdfPage.numberOfCharacters &&
                    pdfPage.characterBounds(at: character).insetBy(dx: -2, dy: -2).contains(start)
                if !hitsText { self.beginFreeTextInput(from: start, to: end, page: page); return }
            }
            guard let engine else { self.setInteractionStatus("新增文字可用；原文编辑需要本机内容编辑组件，原文未修改。"); return }
            self.prepareContentAction(id, start: start, end: end, page: page, engine: engine)
        }
    }

    private func contentEngine() -> ContentEditEngine? {
        ContentEditEngine()
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
                    self.presentInlineTextReplacement(span, page: page, input: input, engine: engine, window: window)
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

    func presentInlineTextReplacement(_ span: ContentSpan, page index: Int, input: PDFEngineInput, engine: ContentEditEngine, window: NSWindow, draft: String? = nil, draftStyle: ContentTextStyle? = nil) {
        guard let page = pdfView.document?.page(at: index), span.bbox.count == 4 else { return }
        let media = page.bounds(for: .mediaBox)
        let height = max(span.bbox[3] - span.bbox[1], span.size * 1.5 + 6)
        let bounds = CGRect(x: media.minX + span.bbox[0], y: media.maxY - span.bbox[1] - height,
                            // The editor has native text/scroll insets; its visual
                            // frame may be wider, but the engine still writes only
                            // the captured original span and enforces neighbor fit.
                            width: min(span.bbox[2] - span.bbox[0] + 12, media.maxX - media.minX - span.bbox[0]), height: height)
        let rgb = draftStyle?.color ?? span.color
        let color = NSColor(srgbRed: CGFloat((rgb >> 16) & 255) / 255,
                            green: CGFloat((rgb >> 8) & 255) / 255, blue: CGFloat(rgb & 255) / 255, alpha: 1)
        let originalFont = NSFont(name: span.font, size: span.size) ?? .systemFont(ofSize: span.size)
        let font = draftStyle.flatMap { style in
            NSFont(name: style.fontName, size: style.size) ?? NSFontManager.shared.convert(originalFont, toSize: style.size)
        } ?? originalFont
        pdfView.inlineTextSession = PDFInlineTextSession(pdfView: pdfView, page: page, bounds: bounds,
            text: draft ?? span.text, font: font,
            color: color, allowsEmptyCommit: true, allowsFractionalFontSize: true, commit: { [weak self, weak window] text, _, font, color in
                guard let self, let window else { throw PDFInlineTextError.staleDocument }
                try self.workspace.validateEngineInput(input)
                guard let srgb = color.usingColorSpace(.sRGB) else { throw OrganizeError.invalidInput }
                let component: (CGFloat) -> Int = { min(255, max(0, Int(($0 * 255).rounded()))) }
                let colorValue = component(srgb.redComponent) << 16 | component(srgb.greenComponent) << 8 | component(srgb.blueComponent)
                // Preserve the original embedded-font fallback when the user
                // only edits text; an explicit style uses the chosen native font.
                let retainsOriginalFont = (draftStyle == nil || draftStyle?.retainsOriginalFont == true) && self.pdfView.inlineTextSession?.hasFontFaceChanges == false
                let styleChanged = !retainsOriginalFont || abs(font.pointSize - span.size) > 0.0001 || colorValue != span.color
                let selectedStyle = styleChanged ? ContentTextStyle(fontName: retainsOriginalFont ? span.font : font.fontName, size: font.pointSize, color: colorValue, retainsOriginalFont: retainsOriginalFont) : nil
                guard text != span.text || selectedStyle != nil else { return }
                guard !text.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
                    throw OrganizeError.processFailed("原文替换暂限单行；多行文字请在空白处新建文本框。草稿已保留。")
                }
                var request = ContentEditRequest(operation: "replace-text", page: index); request.id = span.id; request.text = text
                request.textStyle = selectedStyle
                // Keep failed drafts after the async content engine returns; it
                // never silently falls back to covering the original with an annotation.
                self.runContentEditSheet(request, input: input, engine: engine, window: window) { [weak self, weak window] error in
                    guard let self, let window else { return }
                    self.presentInlineTextReplacement(span, page: index, input: input, engine: engine, window: window, draft: text, draftStyle: selectedStyle)
                    self.setInteractionStatus(Self.contentError(error) + " · 草稿已保留")
                }
            }, failure: { [weak self] error in self?.setInteractionStatus(error.localizedDescription) })
        pdfView.refreshCommittedInkRendering()
        setInteractionStatus("就地修改原文 · 可选字体、字号和颜色；单行、邻文避让 · ⌘↩ 完成 · Esc 取消")
    }

    func presentTextReplacement(_ span: ContentSpan, page: Int, input: PDFEngineInput, engine: ContentEditEngine, window: NSWindow, draft: String? = nil, failure: Error? = nil) {
        let alert = NSAlert(); alert.messageText = "编辑原文文字"
        let details = "原文字体：\(span.font)，\(span.size) 点。使用本机原生字体排版并写入 PDF 字体资源；缺失字体会替代。当前仅支持单行，不压缩或覆盖邻文。"
        alert.informativeText = failure.map { "无法应用：\(Self.contentError($0))\n输入已保留，请调整后重试。\n\n" + details } ?? details
        let field = NSTextField(string: draft ?? span.text); field.frame.size = NSSize(width: 440, height: 30)
        field.setAccessibilityLabel("替换原文文字"); alert.accessoryView = field
        alert.addButton(withTitle: "应用"); alert.addButton(withTitle: "取消")
        alert.window.initialFirstResponder = field
        alert.beginSheetModal(for: window) { [weak self, weak window] response in
            guard let self, let window, response == .alertFirstButtonReturn else { return }
            alert.window.endEditing(for: nil)
            let text = field.stringValue
            var request = ContentEditRequest(operation: "replace-text", page: page); request.id = span.id; request.text = text
            self.runContentEditSheet(request, input: input, engine: engine, window: window) { [weak self, weak window] error in
                guard let self, let window else { return }
                self.presentTextReplacement(span, page: page, input: input, engine: engine, window: window, draft: text, failure: error)
            }
        }
    }

    private func runContentEditSheet(_ request: ContentEditRequest, input: PDFEngineInput, engine: ContentEditEngine, window: NSWindow, onFailure: ((Error) -> Void)? = nil) {
        isContentEditPending = true
        let progress = NSAlert(); progress.messageText = "正在修改 PDF 内容…"; progress.informativeText = "仅在本机处理当前快照。取消会丢弃结果，不覆盖原文件。"
        progress.addButton(withTitle: "取消处理")
        var failure: Error?
        // Show the retry sheet only AFTER AppKit has dismissed progress. A
        // status-bar-only failure otherwise looks like an unresponsive Apply.
        progress.beginSheetModal(for: window) { [weak self] response in
            if response == .alertFirstButtonReturn { self?.cancelLocalTool() }
            else if let failure { onFailure?(failure) }
        }
        Task { [weak self] in
            guard let self else { return }
            do { try await self.applyContentEdit(request, input: input, engine: engine) }
            catch { failure = error; self.setInteractionStatus(Self.contentError(error)) }
            self.isContentEditPending = false
            if progress.window.sheetParent != nil { window.endSheet(progress.window, returnCode: .stop) }
        }
    }
    private static func contentError(_ error: Error) -> String {
        if let e = error as? PDFWorkspaceError { return WorkbenchMessage.error(e) }
        if case .processFailed(let message) = error as? OrganizeError {
            let reason: String
            if message.contains("does not fit") { reason = "新文字超出原文字的可用宽度或页面边界，请缩短文字。" }
            else if message.contains("overlap") || message.contains("Overlapping") { reason = "新文字会与相邻文字、链接或批注重叠，请调整内容。" }
            else if message.contains("glyphs") { reason = "存在当前字体无法写入的字符，请使用常用中文或英文。" }
            else if message.contains("single non-control") { reason = "目前仅支持单行文字，请移除换行或控制字符。" }
            else { reason = message }
            return "内容未修改：" + reason
        }
        if case .cancelled = error as? OrganizeError { return "已取消 · 内容未修改" }
        return "内容未修改：\(error.localizedDescription)"
    }
}
