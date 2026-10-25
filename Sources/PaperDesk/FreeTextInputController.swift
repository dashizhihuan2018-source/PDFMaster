import AppKit
import PDFKit
import PaperCore

@MainActor extension WorkbenchController {
    func beginFreeTextInput(from start: CGPoint, to end: CGPoint, page index: Int) {
        guard workspace.allowsCommenting, !isLocalToolRunning,
              let page = pdfView.document?.page(at: index), pdfView.finishInlineTextEditing() else { return }
        let crop = pdfView.convert(page.bounds(for: .cropBox), from: page)
        let a = pdfView.convert(start, from: page), z = pdfView.convert(end, from: page)
        let scale = max(0.01, pdfView.scaleFactor)
        let dragged = hypot(a.x - z.x, a.y - z.y) >= 4
        let width = min(crop.width, max(60 * scale, dragged ? abs(z.x-a.x) : 240 * scale))
        let height = min(crop.height, max(32 * scale, dragged ? abs(z.y-a.y) : 36 * scale))
        let left = min(max(dragged ? min(a.x,z.x) : a.x, crop.minX), crop.maxX-width)
        let top = min(max(dragged ? max(a.y,z.y) : a.y, crop.minY+height), crop.maxY)
        let bounds = pdfView.convert(CGRect(x: left, y: top-height, width: width, height: height), to: page)
        // A PDF-standard family survives PDFKit's /DA writer, including Bold;
        // private system-font names otherwise round-trip as regular Helvetica.
        startFreeTextEditor(page: index, bounds: bounds, text: "", font: NSFont(name: "Helvetica", size: 14) ?? .systemFont(ofSize: 14), color: .black, annotationIndex: nil)
    }

    func editFreeTextInline(page index: Int, annotationIndex: Int) {
        guard workspace.allowsCommenting, !isLocalToolRunning, pdfView.finishInlineTextEditing(),
              let page = workspace.document?.page(at: index), page.annotations.indices.contains(annotationIndex) else { return }
        let annotation = page.annotations[annotationIndex]
        guard annotation.type == "FreeText" else { return }
        startFreeTextEditor(page: index, bounds: annotation.bounds, text: annotation.contents ?? "",
                            font: annotation.font ?? .systemFont(ofSize: 12), color: annotation.fontColor ?? .black,
                            annotationIndex: annotationIndex)
    }

    private func startFreeTextEditor(page index: Int, bounds: CGRect, text: String, font: NSFont, color: NSColor, annotationIndex: Int?) {
        guard let page = pdfView.document?.page(at: index), let document = workspace.document else { return }
        let revision = workspace.revision
        let annotation = annotationIndex.flatMap { page.annotations.indices.contains($0) ? page.annotations[$0] : nil }
        let rotation = annotation?.value(forAnnotationKey: PDFAnnotationKey(rawValue: "/Rotate")) as? Int ?? page.rotation
        pdfView.inlineTextSession = PDFInlineTextSession(pdfView: pdfView, page: page, bounds: bounds, text: text,
            font: font, color: color, annotation: annotation, commit: { [weak self] text, bounds, font, color in
                guard let self, self.workspace.document === document, self.workspace.revision == revision else { throw PDFInlineTextError.staleDocument }
                if let annotationIndex {
                    try self.workspace.updateFreeText(page: index, index: annotationIndex, text: text, bounds: bounds, font: font, color: color, rotation: rotation)
                } else {
                    try self.workspace.addText(text, page: index, bounds: bounds, font: font, color: color, rotation: rotation)
                }
                do { try self.synchronizeAnnotationPresentation(pages: IndexSet(integer: index)) }
                catch { self.refreshDocument(preservePage: index) }
                self.setInteractionStatus("文字已保存到当前文档 · 可撤销 · 点击其他位置继续输入 · 请存储副本")
            }, failure: { [weak self] error in self?.setInteractionStatus(error.localizedDescription) })
        pdfView.refreshCommittedInkRendering()
        setInteractionStatus("页面内输入 · 回车换行 · ⌘Z 撤销输入 · ⌘↩ / 点击外部完成 · Esc 取消草稿")
    }
}
