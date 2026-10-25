import AppKit
import PaperCore
import PDFKit

@MainActor final class PDFCopilotController: NSWindowController, NSWindowDelegate, NSTextViewDelegate {
    /// Embedded in the reading workspace. No separate Copilot window is created.
    let panelView = NSView(frame: NSRect(x: 0, y: 0, width: 400, height: 700))
    let closeButton = NSButton()
    private let moreButton = NSButton()
    private let settingsPopover = NSPopover()
    private let quickActions = NSStackView()
    var quickActionTitles: [String] { quickActions.arrangedSubviews.compactMap { ($0 as? NSButton)?.title } }
    var onClose: (() -> Void)?
    let consentButton = NSButton(checkboxWithTitle: "发送范围内文字到 DMX（可能计费）", target: nil, action: nil)
    let scopeChoice = NSPopUpButton()
    let providerChoice = NSPopUpButton()
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
    private let agnesKeyProvider: () throws -> String?
    private let deepSeekKeyProvider: () throws -> String?
    private(set) var conversation: [DMXMessage] = []
    private var conversationScope: String?
    var selectedProvider: PDFTextProvider { PDFTextProvider.allCases[max(0, min(providerChoice.indexOfSelectedItem, PDFTextProvider.allCases.count - 1))] }
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
         revisionProvider: @escaping () -> Int = { 0 }, sessionIdentityProvider: @escaping () -> String? = { nil }, client: DMXAIClient = DMXAIClient(),
         agnesKeyProvider: @escaping () throws -> String? = { nil }, initialProvider: PDFTextProvider = .dmx,
         deepSeekKeyProvider: @escaping () throws -> String? = { nil }) {
        self.agnesKeyProvider = agnesKeyProvider
        self.deepSeekKeyProvider = deepSeekKeyProvider
        self.documentProvider = documentProvider; self.currentPageProvider = currentPageProvider; self.keyProvider = keyProvider; self.onNavigate = onNavigate
        self.revisionProvider = revisionProvider; self.identityProvider = sessionIdentityProvider; self.client = client
        super.init(window: nil)
        buildUI(panelView); selectProvider(initialProvider)
    }
    required init?(coder: NSCoder) { fatalError() }
    private func buildUI(_ root: NSView) {
        root.identifier = NSUserInterfaceItemIdentifier("pdf-copilot-sidebar")
        root.wantsLayer = true; root.layer?.backgroundColor = NSColor(srgbRed: 0.085, green: 0.085, blue: 0.10, alpha: 1).cgColor
        root.layer?.masksToBounds = true
        let title = NSTextField(labelWithString: "PDF Copilot"); title.font = .systemFont(ofSize: 12, weight: .semibold)
        let header = NSView(); header.wantsLayer = true; header.layer?.backgroundColor = ShellPalette.top.cgColor
        moreButton.image = NSImage(systemSymbolName: "ellipsis", accessibilityDescription: "PDF Copilot 更多")
        moreButton.setAccessibilityLabel("PDF Copilot 更多"); moreButton.isBordered = false
        moreButton.target = self; moreButton.action = #selector(showMore)
        closeButton.image = NSImage(systemSymbolName: "xmark", accessibilityDescription: "关闭 PDF Copilot")
        closeButton.setAccessibilityLabel("关闭 PDF Copilot"); closeButton.isBordered = false
        closeButton.target = self; closeButton.action = #selector(requestClose)
        [title, moreButton, closeButton].forEach { $0.translatesAutoresizingMaskIntoConstraints = false; header.addSubview($0) }
        NSLayoutConstraint.activate([
            title.leadingAnchor.constraint(equalTo: header.leadingAnchor, constant: 12), title.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            closeButton.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -8), closeButton.centerYAnchor.constraint(equalTo: header.centerYAnchor), closeButton.widthAnchor.constraint(equalToConstant: 20), closeButton.heightAnchor.constraint(equalToConstant: 22),
            moreButton.trailingAnchor.constraint(equalTo: closeButton.leadingAnchor, constant: -6), moreButton.centerYAnchor.constraint(equalTo: header.centerYAnchor), moreButton.widthAnchor.constraint(equalToConstant: 24)
        ])
        let notice = NSTextField(wrappingLabelWithString: "仅在点击生成时向所选服务商发送范围内文字（最多 4 万字）和本轮对话。默认当前页，不上传 PDF 或图片。可能计费；回答和引用需对照原文。更换文档、账户或服务商会清空对话。")
        notice.textColor = .secondaryLabelColor; notice.font = .systemFont(ofSize: 12)
        scopeChoice.addItems(withTitles: ["当前页（默认）", "全文（最多 4 万字，超限截断）"])
        scopeChoice.setAccessibilityLabel("PDF 发送范围")
        providerChoice.addItems(withTitles: PDFTextProvider.allCases.map(\.title)); providerChoice.setAccessibilityLabel("PDF AI 服务商")
        providerChoice.target = self; providerChoice.action = #selector(providerChanged)
        scopeChoice.target = self; scopeChoice.action = #selector(contextChoiceChanged)
        actionChoice.addItems(withTitles: PDFCopilotAction.allCases.map(\.title)); actionChoice.setAccessibilityLabel("PDF Copilot 操作")
        modelField.stringValue = DMXAIClient.defaultTextModel; modelField.placeholderString = "文本模型 ID"; modelField.setAccessibilityLabel("DMX 文本模型")
        questionField.placeholderString = "询问 PDF Copilot…"; questionField.setAccessibilityLabel("PDF 问题")
        questionField.isBordered = false; questionField.drawsBackground = false; questionField.focusRingType = .none
        questionField.font = .systemFont(ofSize: 12); questionField.target = self; questionField.action = #selector(sendQuestion)
        questionField.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        languageField.placeholderString = "翻译目标语言"; languageField.setAccessibilityLabel("翻译目标语言")
        generateButton.title = ""; generateButton.image = NSImage(systemSymbolName: "paperplane.fill", accessibilityDescription: "发送问题（可能计费）")
        generateButton.setAccessibilityLabel("发送问题（可能计费）"); generateButton.toolTip = "发送问题（可能计费）"
        generateButton.isBordered = false; generateButton.target = self; generateButton.action = #selector(sendQuestion)
        cancelButton.title = ""; cancelButton.image = NSImage(systemSymbolName: "stop.fill", accessibilityDescription: "停止请求")
        cancelButton.isBordered = false; cancelButton.setAccessibilityLabel("停止请求"); cancelButton.isHidden = true
        cancelButton.target = self; cancelButton.action = #selector(cancelRequest); cancelButton.isEnabled = false
        let settings = NSButton(title: "AI Key 设置…", target: self, action: #selector(openSettings))
        let newChat = NSButton(title: "新对话", target: self, action: #selector(beginNewChat))
        let copy = NSButton(title: "复制对话", target: self, action: #selector(copyConversation))
        quickActions.orientation = .horizontal; quickActions.spacing = 5; quickActions.distribution = .fillEqually
        for (action, title) in [(PDFCopilotAction.summarize, "总结摘要"), (.keyPoints, "列出要点"), (.annotationSummary, "批注摘要"), (.quiz, "生成小测")] {
            let b = NSButton(title: title, target: self, action: #selector(quickAction(_:))); b.identifier = NSUserInterfaceItemIdentifier(action.rawValue)
            b.font = .systemFont(ofSize: 10); b.bezelStyle = .inline; b.contentTintColor = .systemPurple
            b.heightAnchor.constraint(equalToConstant: 24).isActive = true; quickActions.addArrangedSubview(b)
        }
        answerView.setAccessibilityLabel("PDF Copilot 回答（未核实文本）")
        let scroll = NSScrollView(); scroll.hasVerticalScroller = true; scroll.borderType = .noBorder; scroll.drawsBackground = false
        answerView.isEditable = false; answerView.isSelectable = true; answerView.isRichText = true; answerView.delegate = self
        answerView.drawsBackground = false; answerView.font = .systemFont(ofSize: 15); answerView.textColor = ShellPalette.primary
        answerView.linkTextAttributes = [.foregroundColor: NSColor.systemIndigo, .underlineStyle: 0]
        answerView.textContainerInset = NSSize(width: 12, height: 12); answerView.isVerticallyResizable = true; answerView.isHorizontallyResizable = false
        answerView.autoresizingMask = [.width]; answerView.textContainer?.widthTracksTextView = true
        answerView.textContainer?.containerSize = NSSize(width: 376, height: CGFloat.greatestFiniteMagnitude); scroll.documentView = answerView
        citations.addItem(withTitle: "引用页码（回答后可跳转）"); citations.isEnabled = false
        citations.target = self; citations.action = #selector(navigateCitation)
        citations.controlSize = .mini; citations.font = .systemFont(ofSize: 10); citations.isHidden = true
        let composer = NSView(); composer.wantsLayer = true; composer.layer?.cornerRadius = 7
        composer.layer?.borderWidth = 1; composer.layer?.borderColor = NSColor.separatorColor.cgColor
        composer.layer?.backgroundColor = NSColor(calibratedWhite: 0.12, alpha: 1).cgColor
        [questionField, generateButton, cancelButton].forEach { $0.translatesAutoresizingMaskIntoConstraints = false; composer.addSubview($0) }
        NSLayoutConstraint.activate([
            composer.heightAnchor.constraint(equalToConstant: 36), questionField.leadingAnchor.constraint(equalTo: composer.leadingAnchor, constant: 9), questionField.centerYAnchor.constraint(equalTo: composer.centerYAnchor), questionField.trailingAnchor.constraint(equalTo: generateButton.leadingAnchor, constant: -4),
            generateButton.trailingAnchor.constraint(equalTo: composer.trailingAnchor, constant: -6), generateButton.centerYAnchor.constraint(equalTo: composer.centerYAnchor), generateButton.widthAnchor.constraint(equalToConstant: 24), generateButton.heightAnchor.constraint(equalToConstant: 24),
            cancelButton.centerXAnchor.constraint(equalTo: generateButton.centerXAnchor), cancelButton.centerYAnchor.constraint(equalTo: generateButton.centerYAnchor), cancelButton.widthAnchor.constraint(equalToConstant: 24), cancelButton.heightAnchor.constraint(equalToConstant: 24)
        ])
        consentButton.font = .systemFont(ofSize: 10); consentButton.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        scopeChoice.controlSize = .mini; scopeChoice.font = .systemFont(ofSize: 10)
        let privacy = NSTextField(labelWithString: "仅文字 · 回答需核实"); privacy.font = .systemFont(ofSize: 10); privacy.textColor = .secondaryLabelColor
        let contextRow = NSStackView(views: [privacy, scopeChoice]); contextRow.orientation = .horizontal; contextRow.distribution = .fill; contextRow.spacing = 8
        privacy.setContentHuggingPriority(.defaultLow, for: .horizontal); scopeChoice.widthAnchor.constraint(equalToConstant: 150).isActive = true
        statusLabel.font = .systemFont(ofSize: 10); statusLabel.textColor = .secondaryLabelColor
        statusLabel.maximumNumberOfLines = 2
        let footer = NSStackView(views: [citations, quickActions, composer, consentButton, contextRow, statusLabel])
        footer.orientation = .vertical; footer.alignment = .leading; footer.spacing = 6; footer.detachesHiddenViews = true
        for child in [citations, quickActions, composer, consentButton, contextRow, statusLabel] { child.widthAnchor.constraint(equalTo: footer.widthAnchor).isActive = true }
        let divider = NSBox(); divider.boxType = .separator
        let edge = NSBox(); edge.boxType = .separator
        [header, scroll, divider, footer, edge].forEach { $0.translatesAutoresizingMaskIntoConstraints = false; root.addSubview($0) }
        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: root.topAnchor), header.leadingAnchor.constraint(equalTo: root.leadingAnchor), header.trailingAnchor.constraint(equalTo: root.trailingAnchor), header.heightAnchor.constraint(equalToConstant: 32),
            footer.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 10), footer.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -10), footer.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -8),
            divider.leadingAnchor.constraint(equalTo: root.leadingAnchor), divider.trailingAnchor.constraint(equalTo: root.trailingAnchor), divider.bottomAnchor.constraint(equalTo: footer.topAnchor, constant: -8),
            scroll.topAnchor.constraint(equalTo: header.bottomAnchor), scroll.leadingAnchor.constraint(equalTo: root.leadingAnchor), scroll.trailingAnchor.constraint(equalTo: root.trailingAnchor), scroll.bottomAnchor.constraint(equalTo: divider.topAnchor),
            edge.leadingAnchor.constraint(equalTo: root.leadingAnchor), edge.topAnchor.constraint(equalTo: root.topAnchor), edge.bottomAnchor.constraint(equalTo: root.bottomAnchor), edge.widthAnchor.constraint(equalToConstant: 1)
        ])
        // Provider/model/action configuration is accessed from the reference's overflow position.
        let configuration = NSViewController(); configuration.view = NSView(frame: NSRect(x: 0, y: 0, width: 340, height: 450))
        let actions = NSStackView(views: [settings, newChat, copy]); actions.orientation = .horizontal; actions.spacing = 6
        let runAction = NSButton(title: "运行所选操作（可能计费）", target: self, action: #selector(generate))
        AIWindowLayout.stack([notice, providerChoice, actionChoice, modelField, languageField, runAction, actions], in: configuration.view)
        settingsPopover.contentViewController = configuration; settingsPopover.contentSize = NSSize(width: 340, height: 450); settingsPopover.behavior = .transient
        consentButton.state = .off
    }
    @objc private func showMore() { settingsPopover.show(relativeTo: moreButton.bounds, of: moreButton, preferredEdge: .minX) }
    @objc private func requestClose() { onClose?() }
    @objc private func sendQuestion() {
        guard let index = PDFCopilotAction.allCases.firstIndex(of: .ask) else { return }
        actionChoice.selectItem(at: index); _ = startGeneration()
    }
    @discardableResult func startGeneration() -> Bool {
        guard !isRunning else { return false }
        let provider = selectedProvider
        guard consentButton.state == .on else { statusLabel.stringValue = "请先勾选发送范围内文字到 \(provider.title) 的授权。"; return false }
        let index = actionChoice.indexOfSelectedItem
        guard PDFCopilotAction.allCases.indices.contains(index) else { return false }
        let action = PDFCopilotAction.allCases[index]
        if action == .ask && questionField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { statusLabel.stringValue = "请输入问题。"; return false }
        let key: String
        do {
            let saved: String?
            switch provider {
            case .agnes: saved = try agnesKeyProvider()
            case .dmx: saved = try keyProvider()
            case .deepseek: saved = try deepSeekKeyProvider()
            }
            guard let saved, !saved.isEmpty else { statusLabel.stringValue = "请先登录并在设置中保存 \(provider.title) Key。"; return false }; key = saved
        } catch AccountStoreFailure.loginRequired {
            statusLabel.stringValue = "请先登录本机账户。"; return false
        } catch { statusLabel.stringValue = "无法读取本机 Key，请打开设置检查钥匙串权限。"; return false }
        guard let document = documentProvider() else { statusLabel.stringValue = "请先打开 PDF 文档。"; return false }
        let context: PDFCopilotContext
        do { context = try PDFCopilotContext.extract(document: document, scope: scopeChoice.indexOfSelectedItem == 0 ? .currentPage : .wholeDocument, currentPage: currentPageProvider(), annotationsOnly: action == .annotationSummary) }
        catch { statusLabel.stringValue = (error as? PDFCopilotContextError)?.localizedDescription ?? "无法提取所选范围。"; return false }
        let revision = revisionProvider(); let identity = identityProvider(); let id = UUID(); requestID = id
        let scope = "\(ObjectIdentifier(document))-\(revision)-\(identity ?? "guest")-\(provider.rawValue)-\(scopeChoice.indexOfSelectedItem)-\(scopeChoice.indexOfSelectedItem == 0 ? currentPageProvider() : -1)"
        if conversationScope != scope { conversation = []; answerView.string = ""; conversationScope = scope }
        let prompt = action == .ask ? questionField.stringValue : action.title + (questionField.stringValue.isEmpty ? "" : "：" + questionField.stringValue)
        let messages = context.messages(action: action, question: questionField.stringValue, language: languageField.stringValue, history: action == .ask ? conversation : [])
        let model = modelField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines); let client = self.client
        referencedPages = []; citations.isEnabled = false
        setRunning(true); statusLabel.stringValue = "发送 \(context.pages.count) 页的 \(context.text.count) 字文字\(context.isTruncated ? "（已截断，不代表全文）" : "")，等待 \(provider.title) 回答…"
        requestTask = Task { [weak self] in
            do {
                let answer = try await client.completeText(provider: provider, model: model, messages: messages, key: key)
                guard let self, self.requestID == id else { return }
                guard self.documentProvider() === document, self.revisionProvider() == revision, self.identityProvider() == identity else {
                    self.statusLabel.stringValue = "文档或账户已变化；旧请求结果已忽略。"; self.setRunning(false); return
                }
                self.conversation.append(contentsOf: [DMXMessage(role: "user", content: prompt), DMXMessage(role: "assistant", content: answer)])
                self.conversation = Array(self.conversation.suffix(6))
                self.referencedPages = context.citedPages(in: answer)
                self.answerView.textStorage?.setAttributedString(PDFCopilotRichText.conversation(self.conversation, citedPages: Set(self.referencedPages)))
                self.answerView.scrollToEndOfDocument(nil)
                self.resultDocument = document; self.resultRevision = revision; self.resultIdentity = identity
                self.referencedPages = context.citedPages(in: answer); self.citations.removeAllItems()
                self.citations.addItem(withTitle: "引用页码（需对照原文核验）")
                self.citations.addItems(withTitles: self.referencedPages.map { "第 \($0) 页" }); self.citations.isEnabled = !self.referencedPages.isEmpty
                self.citations.isHidden = self.referencedPages.isEmpty
                self.statusLabel.stringValue = "回答已返回（未核实）\(context.isTruncated ? "；输入已截断，仅覆盖显示范围" : "")。\(context.text.count) 字、\(context.pages.count) 页。"
            } catch {
                guard let self, self.requestID == id else { return }
                if case DMXAIError.http(let status) = error { self.statusLabel.stringValue = "\(provider.title) 请求失败（HTTP \(status)）；请检查 Key、模型权限和额度。未自动重试。" }
                else { self.statusLabel.stringValue = (error as? DMXAIError)?.localizedDescription.replacingOccurrences(of: "DMX", with: provider.title) ?? "AI 请求失败；未自动重试。" }
            }
            guard let self, self.requestID == id else { return }; self.setRunning(false)
        }
        return true
    }
    @objc private func generate() { _ = startGeneration() }
    func selectProvider(_ provider: PDFTextProvider) {
        invalidateContext(); providerChoice.selectItem(at: PDFTextProvider.allCases.firstIndex(of: provider)!)
        modelField.stringValue = provider.defaultModel
        modelField.setAccessibilityLabel("\(provider.title) 文本模型")
        consentButton.title = "允许发送文字及本轮对话到 \(provider.title)（可能计费）"
        statusLabel.stringValue = "\(provider.title) · 仅在点击生成时发送文字；请先在 AI Key 设置中保存对应密钥。"
    }
    @objc private func providerChanged() { selectProvider(selectedProvider) }
    @objc private func contextChoiceChanged() { invalidateContext() }
    @objc func beginNewChat() { invalidateContext(); questionField.stringValue = "" }
    @objc private func quickAction(_ sender: NSButton) {
        guard !isRunning, let action = PDFCopilotAction(rawValue: sender.identifier?.rawValue ?? ""), let index = PDFCopilotAction.allCases.firstIndex(of: action) else { return }
        actionChoice.selectItem(at: index); _ = startGeneration()
    }
    @objc private func copyConversation() {
        guard !answerView.string.isEmpty else { return }
        let raw = conversation.map { ($0.role == "user" ? "你：" : "PDF Copilot：") + "\n" + $0.content }.joined(separator: "\n\n")
        NSPasteboard.general.clearContents(); NSPasteboard.general.setString(raw, forType: .string)
        if let rtf = answerView.textStorage?.rtf(from: NSRange(location: 0, length: answerView.textStorage?.length ?? 0), documentAttributes: [:]) { NSPasteboard.general.setData(rtf, forType: .rtf) }
    }
    func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
        guard let url = link as? URL, url.scheme == "pdfmaster-citation", url.host == "page",
              let page = Int(url.lastPathComponent), referencedPages.contains(page),
              documentProvider() === resultDocument, revisionProvider() == resultRevision, identityProvider() == resultIdentity else { return true }
        onNavigate(page - 1); return true
    }
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
    @objc private func openSettings() { settingsPopover.performClose(nil); onOpenSettings?() }
    private func setRunning(_ running: Bool) {
        isRunning = running; generateButton.isEnabled = !running; cancelButton.isEnabled = running
        generateButton.isHidden = running; cancelButton.isHidden = !running
        scopeChoice.isEnabled = !running; actionChoice.isEnabled = !running; modelField.isEnabled = !running
        providerChoice.isEnabled = !running
    }
    override func close() { settingsPopover.performClose(nil); invalidateContext(); super.close() }
    func windowWillClose(_ notification: Notification) { cancelRequest(); consentButton.state = .off }
    func invalidateContext() {
        cancelRequest(); consentButton.state = .off; answerView.string = ""
        resultDocument = nil; referencedPages = []; resultIdentity = nil
        conversation = []; conversationScope = nil
        citations.removeAllItems(); citations.addItem(withTitle: "引用页码（回答后可跳转）"); citations.isEnabled = false; citations.isHidden = true
        statusLabel.stringValue = "账户或文档已改变，旧回答已清空。请重新确认发送范围。"
    }
}
