import AppKit
import PaperCore

@MainActor final class AIStudioController: NSWindowController, NSWindowDelegate {
    let promptField = NSTextView()
    let consentButton = NSButton(checkboxWithTitle: "发送提示词到 DMX（可能计费）", target: nil, action: nil)
    let taskIDField = NSTextField()
    let statusLabel = NSTextField(wrappingLabelWithString: "尚未请求。图片使用 gpt-image-2.5；视频使用 Seedance 2.5（4 秒，480p）。")
    let modeChoice = NSPopUpButton()
    private let preview = NSImageView()
    private let generateButton = NSButton(title: "生成（可能计费）", target: nil, action: nil)
    private let queryButton = NSButton(title: "查询视频任务", target: nil, action: nil)
    private let insertButton = NSButton(title: "插入画布", target: nil, action: nil)
    private let saveVideoButton = NSButton(title: "下载视频并另存为…", target: nil, action: nil)
    private let cancelButton = NSButton(title: "停止请求", target: nil, action: nil)
    private let keyProvider: () throws -> String?
    private let onImageGenerated: (Data, String) -> Void
    private let onVideoTaskSaved: ((String) throws -> Void)?
    private let identityProvider: () -> String?
    private let client: DMXAIClient
    private var requestTask: Task<Void, Never>?
    private var requestID = UUID()
    private var generatedImage: Data?
    private var imageIdentity: String?
    private var videoIdentity: String?
    private var videoURL: URL?
    var onOpenSettings: (() -> Void)?
    private(set) var isRunning = false
    init(keyProvider: @escaping () throws -> String?, onImageGenerated: @escaping (Data, String) -> Void, restoredVideoTaskID: String? = nil,
         onVideoTaskSaved: ((String) throws -> Void)? = nil, sessionIdentityProvider: @escaping () -> String? = { nil }, client: DMXAIClient = DMXAIClient()) {
        self.keyProvider = keyProvider; self.onImageGenerated = onImageGenerated; self.onVideoTaskSaved = onVideoTaskSaved
        self.identityProvider = sessionIdentityProvider; self.client = client
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 640, height: 690))
        let window = NSWindow(contentRect: root.frame, styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = "PDF Master · AI 创作"; window.contentView = root; window.isReleasedWhenClosed = false
        super.init(window: window); window.delegate = self; window.minSize = NSSize(width: 600, height: 690)
        buildUI(root)
        taskIDField.stringValue = restoredVideoTaskID ?? ""; consentButton.state = .off
    }
    required init?(coder: NSCoder) { fatalError() }
    func showWindow(relativeTo parent: NSWindow?) {
        if let parent { window?.setFrameOrigin(NSPoint(x: parent.frame.midX - 320, y: parent.frame.midY - 345)) } else { window?.center() }
        showWindow(nil); window?.makeKeyAndOrderFront(nil)
    }
    private func buildUI(_ root: NSView) {
        let title = NSTextField(labelWithString: "AI 创作"); title.font = .systemFont(ofSize: 22, weight: .semibold)
        let notice = NSTextField(wrappingLabelWithString: "仅发送你输入的提示词，不发送当前 PDF。生成可能计费；不会自动重试。视频任务在远端异步执行，关闭窗口或停止请求不代表取消远端计费。")
        notice.textColor = .secondaryLabelColor; notice.font = .systemFont(ofSize: 12)
        modeChoice.addItems(withTitles: ["image2.5 · 图片", "Seedance2.5 · 视频"])
        promptField.setAccessibilityLabel("AI 创作提示词")
        let promptScroll = AIWindowLayout.textScroll(promptField, height: 110, editable: true)
        preview.imageScaling = .scaleProportionallyUpOrDown
        preview.setAccessibilityLabel("AI 图片预览")
        taskIDField.placeholderString = "视频任务 ID（自动记住，可粘贴已有任务）"; taskIDField.setAccessibilityLabel("视频任务 ID")
        statusLabel.font = .systemFont(ofSize: 12)
        let settings = NSButton(title: "DMX Key 设置…", target: self, action: #selector(openSettings))
        generateButton.target = self; generateButton.action = #selector(generate)
        queryButton.target = self; queryButton.action = #selector(query)
        insertButton.target = self; insertButton.action = #selector(insertImage); insertButton.isEnabled = false
        saveVideoButton.target = self; saveVideoButton.action = #selector(saveVideo); saveVideoButton.isEnabled = false
        cancelButton.target = self; cancelButton.action = #selector(cancelRequest); cancelButton.isEnabled = false
        let actions = NSStackView(views: [generateButton, cancelButton, settings]); actions.orientation = .horizontal; actions.spacing = 10
        let outputs = NSStackView(views: [insertButton, queryButton, saveVideoButton]); outputs.orientation = .horizontal; outputs.spacing = 10
        AIWindowLayout.stack([title, notice, modeChoice, promptScroll, consentButton, actions, preview, taskIDField, outputs, statusLabel], in: root)
        preview.heightAnchor.constraint(equalToConstant: 180).isActive = true
    }
    private func availableKey() -> String? {
        do {
            guard let key = try keyProvider(), !key.isEmpty else { statusLabel.stringValue = "请先登录并在设置中保存 DMX Key。"; return nil }
            return key
        } catch { statusLabel.stringValue = "无法读取本机 Key。请打开设置检查账户和钥匙串权限。"; return nil }
    }
    @discardableResult func startGeneration() -> Bool {
        guard !isRunning else { return false }
        guard consentButton.state == .on else { statusLabel.stringValue = "请先勾选发送提示词的授权；生成可能计费。"; return false }
        let prompt = promptField.string.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !prompt.isEmpty, prompt.count <= 32_000 else { statusLabel.stringValue = "请输入 1–32000 字提示词。"; return false }
        guard let key = availableKey() else { return false }
        let video = modeChoice.indexOfSelectedItem == 1; let identity = identityProvider(); let id = UUID(); requestID = id
        setRunning(true); statusLabel.stringValue = video ? "正在提交视频任务…提交后不会自动轮询。" : "正在生成图片…"
        let client = self.client; let saveTask = onVideoTaskSaved
        requestTask = Task { [weak self] in
            defer { if let self, self.requestID == id { self.setRunning(false) } }
            do {
                if video {
                    let taskID = try await client.submitVideo(prompt: prompt, key: key)
                    // Persist the original-account task even if UI context changed during submission.
                    var persistenceFailed = false
                    do { try saveTask?(taskID) } catch { persistenceFailed = true }
                    guard let self else { return }
                    guard self.requestID == id, self.identityProvider() == identity else { return }
                    self.taskIDField.stringValue = taskID
                    self.statusLabel.stringValue = persistenceFailed ? "已提交；本机记忆失败。请立即复制任务 ID，避免丢失已付费结果。" : "已提交，任务 ID 已保留。点击“查询视频任务”获取状态；不会自动查询。"
                } else {
                    let data = try await client.generateImage(prompt: prompt, key: key)
                    guard let self, self.requestID == id, self.identityProvider() == identity else { return }
                    self.generatedImage = data; self.imageIdentity = identity
                    self.preview.image = NSImage(data: data); self.insertButton.isEnabled = true
                    self.statusLabel.stringValue = "图片已生成。点击“插入画布”加入当前画布。"
                }
            } catch {
                guard let self, self.requestID == id, self.identityProvider() == identity else { return }
                self.statusLabel.stringValue = (error as? DMXAIError)?.localizedDescription ?? "AI 请求失败；未自动重试。"
            }
            guard let self, self.requestID == id else { return }; self.setRunning(false)
        }
        return true
    }
    @objc private func generate() { _ = startGeneration() }
    @objc private func query() {
        guard !isRunning else { return }
        guard consentButton.state == .on else { statusLabel.stringValue = "请先勾选 DMX 请求授权。"; return }
        let taskID = taskIDField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard DMXAIClient.validTaskID(taskID), let key = availableKey() else { return }
        let id = UUID(); requestID = id; let identity = identityProvider(); let client = self.client
        setRunning(true); statusLabel.stringValue = "正在查询已有任务…不会重新提交生成。"
        requestTask = Task { [weak self] in
            defer { if let self, self.requestID == id { self.setRunning(false) } }
            do {
                let result = try await client.queryVideo(taskID: taskID, key: key)
                guard let self, self.requestID == id, self.identityProvider() == identity else { return }
                switch result {
                case .running: self.statusLabel.stringValue = "任务仍在处理；稍后可手动查询。"
                case .failed: self.statusLabel.stringValue = "视频任务未成功。请检查 DMX 平台任务记录；未重新提交。"
                case .completed(let url): self.videoURL = url; self.videoIdentity = identity; self.saveVideoButton.isEnabled = true; self.statusLabel.stringValue = "视频已生成；点击下载按钮选择新文件保存。"
                }
            } catch {
                guard let self, self.requestID == id, self.identityProvider() == identity else { return }
                self.statusLabel.stringValue = (error as? DMXAIError)?.localizedDescription ?? "视频查询失败。"
            }
            guard let self, self.requestID == id else { return }; self.setRunning(false)
        }
    }
    @objc private func insertImage() {
        guard let data = generatedImage, imageIdentity == identityProvider(), !isRunning else { return }
        onImageGenerated(data, "AI-image2.5"); statusLabel.stringValue = "已发送到当前画布。"
    }
    @objc private func saveVideo() {
        guard !isRunning, let url = videoURL, videoIdentity == identityProvider() else { return }
        let panel = NSSavePanel(); panel.nameFieldStringValue = "AI-Seedance2.5.mp4"; panel.allowedContentTypes = [.mpeg4Movie]
        guard panel.runModal() == .OK, let destination = panel.url else { return }
        let id = UUID(); requestID = id; let identity = identityProvider(); let client = self.client
        setRunning(true); statusLabel.stringValue = "正在下载视频…不会向媒体地址发送 DMX Key。"
        requestTask = Task { [weak self] in
            defer { if let self, self.requestID == id { self.setRunning(false) } }
            do {
                let bytes = try await client.downloadVideo(url: url)
                guard let self, self.requestID == id, self.identityProvider() == identity else { return }
                try bytes.write(to: destination, options: .atomic); self.statusLabel.stringValue = "视频已另存为：\(destination.lastPathComponent)"
            } catch {
                guard let self, self.requestID == id else { return }; self.statusLabel.stringValue = "视频下载或保存失败；未自动重试。"
            }
            guard let self, self.requestID == id else { return }; self.setRunning(false)
        }
    }
    @objc private func cancelRequest() {
        requestID = UUID(); requestTask?.cancel(); requestTask = nil; setRunning(false)
        statusLabel.stringValue = "已停止本机请求。已提交的远端任务仍可能计费；已有任务 ID 保持不变。"
    }
    @objc private func openSettings() { onOpenSettings?() }
    private func setRunning(_ running: Bool) {
        isRunning = running; generateButton.isEnabled = !running; queryButton.isEnabled = !running
        cancelButton.isEnabled = running; modeChoice.isEnabled = !running; promptField.isEditable = !running
    }
    func windowWillClose(_ notification: Notification) { cancelRequest(); consentButton.state = .off }
    func invalidateContext() {
        // Leave a submitted video request alive so its original-account task ID can be saved.
        consentButton.state = .off; generatedImage = nil; imageIdentity = nil
        videoURL = nil; videoIdentity = nil; preview.image = nil; taskIDField.stringValue = ""
        insertButton.isEnabled = false; saveVideoButton.isEnabled = false
        statusLabel.stringValue = "账户或画布已改变，旧结果已作废。请从当前画布重新打开 AI 创作。"
    }
}

@MainActor enum AIWindowLayout {
    static func textScroll(_ text: NSTextView, height: CGFloat, editable: Bool) -> NSScrollView {
        let scroll = NSScrollView(); scroll.borderType = .bezelBorder; scroll.hasVerticalScroller = true
        text.isEditable = editable; text.isSelectable = true; text.isRichText = false; text.font = .systemFont(ofSize: 13)
        text.textContainerInset = NSSize(width: 8, height: 8); text.isVerticallyResizable = true; text.isHorizontallyResizable = false
        text.autoresizingMask = [.width]; text.textContainer?.widthTracksTextView = true; text.textContainer?.containerSize = NSSize(width: 600, height: CGFloat.greatestFiniteMagnitude)
        scroll.documentView = text; scroll.heightAnchor.constraint(equalToConstant: height).isActive = true
        return scroll
    }
    static func stack(_ views: [NSView], in root: NSView) {
        let scroll = NSScrollView(frame: root.bounds); scroll.hasVerticalScroller = true; scroll.autoresizingMask = [.width, .height]
        let document = AIWindowContentView(frame: root.bounds); document.translatesAutoresizingMaskIntoConstraints = false
        scroll.documentView = document; root.addSubview(scroll)
        let stack = NSStackView(views: views); stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false; document.addSubview(stack)
        NSLayoutConstraint.activate([document.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor), stack.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 22), stack.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -22), stack.topAnchor.constraint(equalTo: document.topAnchor, constant: 20), stack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -20)])
        for view in views { view.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true }
    }
}

private final class AIWindowContentView: NSView { override var isFlipped: Bool { true } }
