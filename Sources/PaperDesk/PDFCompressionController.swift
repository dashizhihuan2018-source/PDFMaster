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
        if let existing = compressionPanelController { existing.showWindow(nil); existing.window?.makeKeyAndOrderFront(nil); return }
        do {
            let panel = try PDFCompressionPanelController(workbench: self, parent: window, qpdf: qpdf)
            compressionPanelController = panel
            panel.onClose = { [weak self, weak panel] in
                if self?.compressionPanelController === panel { self?.compressionPanelController = nil }
            }
            panel.present()
        } catch { setInteractionStatus(Self.localToolError(error)) }
    }
}
