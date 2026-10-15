import AppKit
import PDFKit
import PaperCore

@MainActor extension WorkbenchController {
    func mergeDroppedFiles(_ urls: [URL], at index: Int) async throws {
        guard !isLocalToolRunning else { throw FilePDFImportError.failed("导入", "另一个工具仍在运行") }
        let flag = LocalToolCancellation(); localToolCancellation = flag
        let activity = LocalToolActivity.register(flag)
        try await finishFileMerge(urls, at: index, flag: flag, activity: activity)
    }

    @discardableResult func queueFileMerge(_ urls: [URL], at index: Int) -> Bool {
        guard !isLocalToolRunning, workspace.document != nil else {
            setInteractionStatus("已有导入任务正在进行，或尚未打开 PDF；请稍后重试。")
            return false
        }
        let flag = LocalToolCancellation(); localToolCancellation = flag
        let activity = LocalToolActivity.register(flag)
        Task { [weak self] in
            guard let self else { LocalToolActivity.finish(activity); return }
            do { try await self.finishFileMerge(urls, at: index, flag: flag, activity: activity) }
            catch {
                self.setInteractionStatus(error.localizedDescription)
                if !flag.isCancelled { self.perform { throw error } }
            }
        }
        return true
    }

    private func finishFileMerge(_ urls: [URL], at index: Int, flag: LocalToolCancellation, activity: UUID) async throws {
        setFileImportProgress(true)
        defer { localToolCancellation = nil; LocalToolActivity.finish(activity); setFileImportProgress(false) }
        guard let current = workspace.document, workspace.allowsDocumentAssembly,
              index >= 0, index <= current.pageCount else { throw PDFWorkspaceError.invalidPage }
        let revision = workspace.revision, count = current.pageCount
        setInteractionStatus("正在本地转换 \(urls.count) 个文件… 纯文字 DOCX/RTF 将重新分页；不修改原文件。")
        let prepared = try await FileImportJob.prepare(urls, cancellation: flag)
        guard !flag.isCancelled else { throw FilePDFImportError.cancelled }
        guard workspace.document === current, workspace.revision == revision else { throw PDFWorkspaceError.staleEngineResult }
        try workspace.insertPDFData(prepared, at: index)
        let inserted = (workspace.document?.pageCount ?? count) - count
        refreshDocument(preservePage: index, scale: pdfView.scaleFactor)
        if inserted > 0 { pageOverview.restoreSelection(IndexSet(integersIn: index..<(index + inserted))) }
        setInteractionStatus("已插入 \(urls.count) 个文件，共 \(inserted) 页 · 可一次撤销 · 请存储副本导出合并 PDF")
    }
}

enum FileImportJob {
    static func prepare(_ urls: [URL], cancellation: LocalToolCancellation) async throws -> [Data] {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                DispatchQueue.global(qos: .userInitiated).async {
                    do { continuation.resume(returning: try FilePDFImport.prepare(urls, cancelled: { cancellation.isCancelled })) }
                    catch { continuation.resume(throwing: error) }
                }
            }
        } onCancel: { cancellation.cancel() }
    }
}
