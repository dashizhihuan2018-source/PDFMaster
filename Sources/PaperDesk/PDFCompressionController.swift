import AppKit
import PaperCore
import UniformTypeIdentifiers

struct PDFCompressionPreview {
    let input: PDFEngineInput
    let data: Data
    var before: Int { input.data.count }
    var after: Int { data.count }
    var reduced: Bool { after < before }
    var summary: String {
        let old = ByteCountFormatter.string(fromByteCount: Int64(before), countStyle: .file)
        let new = ByteCountFormatter.string(fromByteCount: Int64(after), countStyle: .file)
        guard reduced else { return "当前设置未能进一步缩小文件。原文件和编辑内容均保持不变。" }
        let percent = 100 * Double(before - after) / Double(before)
        return String(format: "%@ → %@ · 减小 %.1f%%", old, new, percent)
    }
}

@MainActor extension WorkbenchController {
    @objc func reduceFileSize(_ sender: Any?) { presentLocalTool("compress") }

    func prepareCompression(preset: PDFCompressionPreset, qpdf: URL) async throws -> PDFCompressionPreview {
        guard !isLocalToolRunning else { throw OrganizeError.unavailable }
        let input = try workspace.engineInput()
        let flag = LocalToolCancellation(); localToolCancellation = flag
        let activity = LocalToolActivity.register(flag)
        defer { localToolCancellation = nil; LocalToolActivity.finish(activity) }
        let result = try await LocalToolJob.run(data: input.data, cancellation: flag) { source, output, flag in
            let directory = source.deletingLastPathComponent()
            let engine = OrganizeEngine(qpdf: qpdf, scratch: directory)
            for (index, raw) in input.safetySourceDatas.enumerated() {
                let safety = directory.appendingPathComponent("admitted-\(index).pdf")
                try raw.write(to: safety)
                try engine.validateCompressionInput(safety, cancelled: { flag.isCancelled })
            }
            _ = try engine.compress(source, output: output, preset: preset, cancelled: { flag.isCancelled })
            return ""
        }
        guard !flag.isCancelled else { throw OrganizeError.cancelled }
        try workspace.validateEngineInput(input)
        return PDFCompressionPreview(input: input, data: result.data)
    }

    func exportCompression(_ preview: PDFCompressionPreview, to destination: URL) throws {
        try workspace.validateEngineInput(preview.input)
        guard preview.reduced, destination.isFileURL, destination.pathExtension.lowercased() == "pdf" else {
            throw OrganizeError.invalidInput
        }
        let target = destination.standardizedFileURL.resolvingSymlinksInPath()
        guard workspace.sourceURL?.standardizedFileURL.resolvingSymlinksInPath() != target else {
            throw OrganizeError.outputExists
        }
        guard !FileManager.default.fileExists(atPath: destination.path) else { throw OrganizeError.outputExists }
        let staged = destination.deletingLastPathComponent().appendingPathComponent(".PDFMaster-compressed-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: staged) }
        guard FileManager.default.createFile(atPath: staged.path, contents: preview.data, attributes: [.posixPermissions: 0o600]) else {
            throw CocoaError(.fileWriteUnknown)
        }
        // Exclusive publication preserves the selected destination if it appeared during export.
        try FileManager.default.linkItem(at: staged, to: destination)
        setInteractionStatus("压缩副本已保存 · " + preview.summary)
    }

    func presentCompressionOptions(qpdf: URL, window: NSWindow) {
        let choices = NSPopUpButton(frame: NSRect(x: 0, y: 0, width: 360, height: 28))
        choices.addItems(withTitles: PDFCompressionPreset.allCases.map(\.title))
        choices.selectItem(at: 2)
        choices.setAccessibilityLabel("压缩质量")
        let alert = NSAlert(); alert.messageText = "压缩文件大小"
        alert.informativeText = "在本机处理，保留原文件与当前编辑。无损优化不改变画质；其他选项以 JPEG 优化支持的图片，不将页面转成图片。处理后显示实际大小，再由你保存副本。已优化的图片可能无法继续缩小。"
        alert.accessoryView = choices
        alert.addButton(withTitle: "开始压缩"); alert.addButton(withTitle: "取消")
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn,
                  PDFCompressionPreset.allCases.indices.contains(choices.indexOfSelectedItem) else { return }
            self.runCompression(preset: PDFCompressionPreset.allCases[choices.indexOfSelectedItem], qpdf: qpdf, window: window)
        }
    }

    private func runCompression(preset: PDFCompressionPreset, qpdf: URL, window: NSWindow) {
        let progress = NSAlert(); progress.messageText = "正在压缩 PDF…"
        progress.informativeText = "本机处理，不上传文件。取消会丢弃本次结果。"
        progress.addButton(withTitle: "取消处理")
        let indicator = NSProgressIndicator(frame: NSRect(x: 0, y: 0, width: 360, height: 16))
        indicator.isIndeterminate = true; indicator.style = .bar; indicator.startAnimation(nil)
        progress.accessoryView = indicator
        progress.beginSheetModal(for: window) { [weak self] response in
            if response == .alertFirstButtonReturn { self?.cancelLocalTool() }
        }
        Task { [weak self] in
            guard let self else { return }
            do {
                let preview = try await self.prepareCompression(preset: preset, qpdf: qpdf)
                indicator.stopAnimation(nil)
                if progress.window.sheetParent != nil { window.endSheet(progress.window, returnCode: .stop) }
                guard self.view.window === window else { return }
                let result = NSAlert(); result.messageText = preview.reduced ? "压缩完成" : "文件已充分优化"
                result.informativeText = preview.summary + "\n大小基于当前编辑快照。压缩副本保留文字与批注，原文件不变。"
                result.addButton(withTitle: preview.reduced ? "保存压缩副本…" : "好")
                if preview.reduced { result.addButton(withTitle: "取消") }
                result.beginSheetModal(for: window) { [weak self] response in
                    guard let self, preview.reduced, response == .alertFirstButtonReturn else { return }
                    self.chooseCompressionDestination(preview, window: window)
                }
            } catch {
                indicator.stopAnimation(nil)
                if progress.window.sheetParent != nil { window.endSheet(progress.window, returnCode: .stop) }
                self.setInteractionStatus(Self.localToolError(error))
            }
        }
    }

    private func chooseCompressionDestination(_ preview: PDFCompressionPreview, window: NSWindow) {
        let panel = NSSavePanel(); panel.allowedContentTypes = [.pdf]
        panel.title = "保存压缩副本"
        panel.message = "请选择新的文件名，保留原文件。"
        panel.nameFieldStringValue = (workspace.sourceURL?.deletingPathExtension().lastPathComponent ?? "未命名") + "-压缩.pdf"
        panel.directoryURL = workspace.sourceURL?.deletingLastPathComponent()
        panel.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .OK, let url = panel.url else { return }
            do { try self.exportCompression(preview, to: url) }
            catch {
                let failure = NSAlert(); failure.messageText = "未保存压缩副本"
                failure.informativeText = error as? OrganizeError == .outputExists
                    ? "为保护现有文件，请选择新的文件名。"
                    : Self.localToolError(error)
                failure.addButton(withTitle: "好"); failure.beginSheetModal(for: window)
            }
        }
    }
}
