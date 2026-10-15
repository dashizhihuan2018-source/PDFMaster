import AppKit
import PaperCore
import PDFKit

@MainActor final class PDFCopilotController: NSWindowController, NSWindowDelegate {
    let consentButton = NSButton(checkboxWithTitle: "发送范围内文字到 DMX（可能计费）", target: nil, action: nil)
    let scopeChoice = NSPopUpButton()
    let actionChoice = NSPopUpButton()
    let answerView = NSTextView()
    let modelField = NSTextField()
    let questionField = NSTextField()
    let statusLabel = NSTextField(wrappingLabelWithString: "仅在点击生成时提取文字。不会上传 PDF 文件或页面图片。")
    private let languageField = NSTextField(string: "中文")
    private let citations = NSPopUpButton()
    private let generateButton = NSButton(title: "生成（可能计费）", target: nil, action: nil)
    private let cancelButton = NSButton(title: "停止请求", target: nil, action: nil)
    private let documentProvider: () -> PDFDocument?
    private let currentPageProvider: () -> Int
    private let revisionProvider: () -> Int
    private let identityProvider: () -> String?
    private let keyProvider: () throws -> String?
    private let onNavigate: (Int) -> Void
    private let client: DMXAIClient
    private var requestTask: Task<Void, Never>?
    private var requestID = UUID()
    private var resultDocument: PDFDocument?
    private var resultRevision = 0
    private var resultIdentity: String?
    private var referencedPages: [Int] = []
    private(set) var isRunning = false
    var onOpenSettings: (() -> Void)?
    init(documentProvider: @escaping () -> PDFDocument?, currentPageProvider: @escaping () -> Int, keyProvider: @escaping () throws -> String?, onNavigate: @escaping (Int) -> Void,
         revisionProvider: @escaping () -> Int = { 0 }, sessionIdentityProvider: @escaping () -> String? = { nil }, client: DMXAIClient = DMXAIClient()) {
        self.documentProvider = documentProvider; self.currentPageProvider = currentPageProvider; self.keyProvider = keyProvider; self.onNavigate = onNavigate
        self.revisionProvider = revisionProvider; self.identityProvider = sessionIdentityProvider; self.client = client
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 680, height: 740))
        let window = NSWindow(contentRect: root.frame, styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = "PDF Master · PDF Copilot"; window.contentView = root; window.isReleasedWhenClosed = false
        super.init(window: window); window.delegate = self; window.minSize = NSSize(width: 640, height: 740)
        buildUI(root)
    }
    required init?(coder: NSCoder) { fatalError() }
    func showWindow(relativeTo parent: NSWindow?) {
        if let parent { window?.setFrameOrigin(NSPoint(x: parent.frame.midX - 340, y: parent.frame.midY - 370)) } else { window?.center() }
        showWindow(nil); window?.makeKeyAndOrderFront(nil)
    }
    private func buildUI(_ root: NSView) {
        let title = NSTextField(labelWithString: "PDF Copilot"); title.font = .systemFont(ofSize: 22, weight: .semibold)
        let notice = NSTextField(wrappingLabelWithString: "仅发送所选范围的可复制文字（最多 4 万字）。默认当前页，不包含 PDF 二进制、图片或其他文档。DMX 可能计费；回答仅供参考，请核对页码与原文。")
        notice.textColor = .secondaryLabelColor; notice.font = .systemFont(ofSize: 12)
        scopeChoice.addItems(withTitles: ["当前页（默认）", "全文（最多 4 万字，超限截断）"])
        scopeChoice.setAccessibilityLabel("PDF 发送范围")
        actionChoice.addItems(withTitles: PDFCopilotAction.allCases.map(\.title)); actionChoice.setAccessibilityLabel("PDF Copilot 操作")
        modelField.stringValue = DMXAIClient.defaultTextModel; modelField.placeholderString = "DMX 文本模型 ID"; modelField.setAccessibilityLabel("DMX 文本模型")
        questionField.placeholderString = "问题或解释重点（提问时必填）"; questionField.setAccessibilityLabel("PDF 问题")
        languageField.placeholderString = "翻译目标语言"; languageField.setAccessibilityLabel("翻译目标语言")
        generateButton.target = self; generateButton.action = #selector(generate)
        cancelButton.target = self; cancelButton.action = #selector(cancelRequest); cancelButton.isEnabled = false
        let settings = NSButton(title: "DMX Key 设置…", target: self, action: #selector(openSettings))
        let actions = NSStackView(views: [generateButton, cancelButton, settings]); actions.orientation = .horizontal; actions.spacing = 12
        answerView.setAccessibilityLabel("PDF Copilot 回答（未核实文本）")
        let scroll = AIWindowLayout.textScroll(answerView, height: 310, editable: false)
        citations.addItem(withTitle: "引用页码（回答后可跳转）"); citations.isEnabled = false
        citations.target = self; citations.action = #selector(navigateCitation)
        statusLabel.font = .systemFont(ofSize: 12)
        AIWindowLayout.stack([title, notice, scopeChoice, actionChoice, modelField, questionField, languageField, consentButton, actions, scroll, citations, statusLabel], in: root)
        consentButton.state = .off
    }
    @discardableResult func startGeneration() -> Bool {
        guard !isRunning else { return false }
        guard consentButton.state == .on else { statusLabel.stringValue = "请先勾选发送范围内文字到 DMX 的授权。"; return false }
        let index = actionChoice.indexOfSelectedItem
        guard PDFCopilotAction.allCases.indices.contains(index) else { return false }
        let action = PDFCopilotAction.allCases[index]
        if action == .ask && questionField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { statusLabel.stringValue = "请输入问题。"; return false }
        let key: String
        do {
            guard let saved = try keyProvider(), !saved.isEmpty else { statusLabel.stringValue = "请先登录并在设置中保存 DMX Key。"; return false }; key = saved
        } catch { statusLabel.stringValue = "无法读取本机 Key，请打开设置检查钥匙串权限。"; return false }
        guard let document = documentProvider() else { statusLabel.stringValue = "请先打开 PDF 文档。"; return false }
        let context: PDFCopilotContext
        do { context = try PDFCopilotContext.extract(document: document, scope: scopeChoice.indexOfSelectedItem == 0 ? .currentPage : .wholeDocument, currentPage: currentPageProvider(), annotationsOnly: action == .annotationSummary) }
        catch { statusLabel.stringValue = (error as? PDFCopilotContextError)?.localizedDescription ?? "无法提取所选范围。"; return false }
        let revision = revisionProvider(); let identity = identityProvider(); let id = UUID(); requestID = id
        let messages = context.messages(action: action, question: questionField.stringValue, language: languageField.stringValue)
        let model = modelField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines); let client = self.client
        answerView.string = ""; referencedPages = []; citations.isEnabled = false
        setRunning(true); statusLabel.stringValue = "发送 \(context.pages.count) 页的 \(context.text.count) 字文字\(context.isTruncated ? "（已截断，不代表全文）" : "")，等待 DMX 回答…"
        requestTask = Task { [weak self] in
            do {
                let answer = try await client.completeText(model: model, messages: messages, key: key)
                guard let self, self.requestID == id else { return }
                guard self.documentProvider() === document, self.revisionProvider() == revision, self.identityProvider() == identity else {
                    self.statusLabel.stringValue = "文档或账户已变化；旧请求结果已忽略。"; self.setRunning(false); return
                }
                self.answerView.string = answer; self.resultDocument = document; self.resultRevision = revision; self.resultIdentity = identity
                self.referencedPages = context.citedPages(in: answer); self.citations.removeAllItems()
                self.citations.addItem(withTitle: "引用页码（需对照原文核验）")
                self.citations.addItems(withTitles: self.referencedPages.map { "第 \($0) 页" }); self.citations.isEnabled = !self.referencedPages.isEmpty
                self.statusLabel.stringValue = "回答已返回（未核实）\(context.isTruncated ? "；输入已截断，仅覆盖显示范围" : "")。\(context.text.count) 字、\(context.pages.count) 页。"
            } catch {
                guard let self, self.requestID == id else { return }
                self.statusLabel.stringValue = (error as? DMXAIError)?.localizedDescription ?? "AI 请求失败；未自动重试。"
            }
            guard let self, self.requestID == id else { return }; self.setRunning(false)
        }
        return true
    }
    @objc private func generate() { _ = startGeneration() }
    @objc private func navigateCitation() {
        let index = citations.indexOfSelectedItem - 1
        guard referencedPages.indices.contains(index), documentProvider() === resultDocument,
              revisionProvider() == resultRevision, identityProvider() == resultIdentity else { return }
        onNavigate(referencedPages[index] - 1)
    }
    @objc private func cancelRequest() {
        requestID = UUID(); requestTask?.cancel(); requestTask = nil; setRunning(false)
        statusLabel.stringValue = "已停止本机请求；已发送内容无法撤回，远端仍可能计费。"
    }
    @objc private func openSettings() { onOpenSettings?() }
    private func setRunning(_ running: Bool) {
        isRunning = running; generateButton.isEnabled = !running; cancelButton.isEnabled = running
        scopeChoice.isEnabled = !running; actionChoice.isEnabled = !running; modelField.isEnabled = !running
    }
    func windowWillClose(_ notification: Notification) { cancelRequest(); consentButton.state = .off }
    func invalidateContext() {
        cancelRequest(); consentButton.state = .off; answerView.string = ""
        resultDocument = nil; referencedPages = []; resultIdentity = nil
        citations.removeAllItems(); citations.addItem(withTitle: "引用页码（回答后可跳转）"); citations.isEnabled = false
        statusLabel.stringValue = "账户或文档已改变，旧回答已清空。请重新确认发送范围。"
    }
}
