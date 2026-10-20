import AppKit
import WebKit
import CryptoKit

/// Bundled InfiniteCanvas UI. No website navigation or automatic document upload.
@MainActor
final class CanvasWebsiteController: NSViewController, WKNavigationDelegate, WKUIDelegate, WKDownloadDelegate {
    static let startURL = URL(string: "pdfmaster-canvas://workspace/canvas?mode=recent")!
    private(set) var webView: WKWebView!
    private let status = NSTextField(labelWithString: "无限画布 · 软件内置 · 本机保存")
    let nativeBridge: CanvasNativeBridge
    private var consentGranted = false
    private var consentTask: Task<Bool, Never>?
    private var downloads: [ObjectIdentifier: WKDownload] = [:]
    private var exportPopups: [ObjectIdentifier: WKWebView] = [:]

    init(bridge: CanvasNativeBridge? = nil) { nativeBridge = bridge ?? CanvasNativeBridge(); super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        let root = NSView(); root.wantsLayer = true
        root.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        let config = WKWebViewConfiguration()
        config.websiteDataStore = WKWebsiteDataStore(forIdentifier: Self.dataStoreID(Bundle.main.bundleIdentifier ?? "cn.nankong.pdfmaster.desktop"))
        config.setURLSchemeHandler(CanvasBundleScheme(), forURLScheme: "pdfmaster-canvas")
        config.userContentController.addScriptMessageHandler(nativeBridge, contentWorld: .page, name: "pdfmasterCanvas")
        nativeBridge.onSettings = { [weak self] in self?.showKeySettings() }
        nativeBridge.onExport = { [weak self] data, name in await self?.saveExport(data, name: name) ?? false }
        nativeBridge.confirmSend = { [weak self] description in await self?.confirmGeneration(description) ?? false }
        webView = WKWebView(frame: .zero, configuration: config)
        webView.navigationDelegate = self; webView.uiDelegate = self
        webView.allowsBackForwardNavigationGestures = true
        webView.setAccessibilityLabel("无限画布 · 软件内置 · 离线编辑，生成时联网")
        status.font = .systemFont(ofSize: 11); status.textColor = .secondaryLabelColor
        status.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let bar = NSStackView(); bar.orientation = .horizontal; bar.spacing = 12
        bar.edgeInsets = NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 10)
        let reload = NSButton(title: "返回画布", target: self, action: #selector(reloadWebsite))
        let settings = NSButton(title: "DMX Key 设置…", target: self, action: #selector(showKeySettings))
        let help = NSButton(title: "使用教程", target: self, action: #selector(showTutorial))
        for button in [reload, settings, help] { button.bezelStyle = .inline; button.font = .systemFont(ofSize: 11) }
        for child in [status, NSView(), reload, settings, help] { bar.addArrangedSubview(child) }
        for child in [bar, webView!] { child.translatesAutoresizingMaskIntoConstraints = false; root.addSubview(child) }
        NSLayoutConstraint.activate([
            bar.topAnchor.constraint(equalTo: root.topAnchor), bar.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            bar.trailingAnchor.constraint(equalTo: root.trailingAnchor), bar.heightAnchor.constraint(equalToConstant: 30),
            webView.topAnchor.constraint(equalTo: bar.bottomAnchor), webView.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            webView.trailingAnchor.constraint(equalTo: root.trailingAnchor), webView.bottomAnchor.constraint(equalTo: root.bottomAnchor)
        ])
        view = root
        webView.load(URLRequest(url: Self.startURL))
    }

    static func isWebsiteURL(_ url: URL) -> Bool {
        url.scheme == startURL.scheme && url.host == startURL.host && url.port == nil && url.user == nil && url.password == nil
    }
    static func dataStoreID(_ namespace: String) -> UUID {
        let hex = SHA256.hash(data: Data((namespace + ".embedded-canvas-v1").utf8)).prefix(16).map { String(format: "%02x", $0) }.joined()
        let parts = [String(hex.prefix(8)), String(hex.dropFirst(8).prefix(4)), String(hex.dropFirst(12).prefix(4)), String(hex.dropFirst(16).prefix(4)), String(hex.dropFirst(20))]
        return UUID(uuidString: parts.joined(separator: "-"))!
    }
    @objc private func reloadWebsite() {
        consentGranted = false; nativeBridge.revoke()
        webView.load(URLRequest(url: Self.startURL))
    }
    @objc func showTutorial() { webView.load(URLRequest(url: URL(string: "pdfmaster-canvas://workspace/tutorial.html")!)) }
    @objc func showKeySettings() {
        let alert = NSAlert(); alert.messageText = "无限画布 · DMX Key"
        alert.informativeText = "只需填写你自己的 DMX API Key，无需登录网站。Key 仅保存在这台 Mac 的钥匙串，不进入画布、导出文件或网页存储。\n生成内容会发送至 DMX，费用由你的 DMX 账户承担。"
        let field = NSSecureTextField(frame: NSRect(x: 0, y: 0, width: 370, height: 26)); field.placeholderString = "粘贴 DMX API Key（留空不会覆盖）"
        field.setAccessibilityLabel("无限画布 DMX API Key")
        alert.accessoryView = field
        alert.addButton(withTitle: "保存到钥匙串"); alert.addButton(withTitle: "取消"); alert.addButton(withTitle: "移除 Key")
        do { if try nativeBridge.readKey() != nil { alert.informativeText += "\n当前：已保存 Key（隐藏）。" } }
        catch { status.stringValue = "钥匙串暂不可用；请检查本机权限。" }
        let response = alert.runModal()
        defer { field.stringValue = "" }
        do {
            if response == .alertFirstButtonReturn {
                try nativeBridge.saveKey(field.stringValue); consentGranted = false
                status.stringValue = "Key 已保存到钥匙串 · 点击生成时才联网 · 可查看使用教程"
            } else if response == .alertThirdButtonReturn {
                try nativeBridge.removeKey(); consentGranted = false; status.stringValue = "Key 已移除；离线编辑不受影响"
            }
        } catch { status.stringValue = (error as? AccountStoreFailure)?.errorDescription ?? "Key 保存失败；请检查钥匙串权限。" }
    }
    private func confirmGeneration(_ description: String) async -> Bool {
        if consentGranted { return true }
        if let consentTask { return await consentTask.value }
        let task = Task { @MainActor [weak self] () -> Bool in
            guard let self, let window = self.view.window else { return false }
            let alert = NSAlert(); alert.messageText = "允许本次画布会话连接 DMX？"
            alert.informativeText = description + "\n\n允许后，本次画布会话内由你点击的生成、批量生成与任务查询将连接 DMX。请检查生成数量与参数。软件不会上传当前 PDF；只有你选择的素材会随生成请求发送。刷新画布、改 Key 或退出软件后需重新确认。"
            alert.addButton(withTitle: "允许本次会话"); alert.addButton(withTitle: "取消")
            return await withCheckedContinuation { continuation in
                alert.beginSheetModal(for: window) { continuation.resume(returning: $0 == .alertFirstButtonReturn) }
            }
        }
        consentTask = task
        let allowed = await task.value; consentTask = nil; consentGranted = allowed; return allowed
    }

    private func saveExport(_ data: Data, name: String) async -> Bool {
        guard let window = view.window else { return false }
        let panel = NSSavePanel(); panel.nameFieldStringValue = name
        let response = await withCheckedContinuation { continuation in
            panel.beginSheetModal(for: window) { continuation.resume(returning: $0) }
        }
        guard response == .OK, let url = panel.url else { return false }
        do { try data.write(to: url, options: .atomic); return true }
        catch { status.stringValue = "导出失败，请检查保存位置与磁盘空间。"; return false }
    }

    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = action.request.url else { decisionHandler(.cancel); return }
        if action.shouldPerformDownload { decisionHandler(.download); return }
        // FileSaver's macOS WebView fallback writes a data URL into its blank popup.
        if exportPopups[ObjectIdentifier(webView)] != nil, ["data", "blob"].contains(url.scheme ?? "") {
            decisionHandler(.download); return
        }
        // Subframe/resource requests remain under WebKit's origin policy.
        if Self.isWebsiteURL(url) || url.absoluteString == "about:blank" || (action.targetFrame?.isMainFrame == false && ["data", "blob"].contains(url.scheme ?? "")) {
            decisionHandler(.allow); return
        }
        decisionHandler(.cancel)
        // External links never turn this workspace into a website or open a browser.
    }
    func webView(_ webView: WKWebView, decidePolicyFor response: WKNavigationResponse, decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void) {
        decisionHandler(response.canShowMIMEType ? .allow : .download)
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        status.stringValue = "无限画布 · 软件内置 · 本机保存 · 仅 AI 生成联网"
    }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { show(error) }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { show(error) }
    private func show(_ error: Error) {
        guard (error as NSError).code != NSURLErrorCancelled else { return }
        status.stringValue = "内置画布加载失败；请返回画布重试，或重新安装完整应用。"
    }
    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) { consentGranted = false; nativeBridge.revoke(); status.stringValue = "画布进程已停止；点击返回画布恢复本机存档。" }
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for action: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        if action.request.url?.absoluteString.isEmpty != false || action.request.url?.absoluteString == "about:blank" {
            let popup = WKWebView(frame: .zero, configuration: configuration)
            popup.navigationDelegate = self; popup.uiDelegate = self
            exportPopups[ObjectIdentifier(popup)] = popup
            return popup
        }
        if let url = action.request.url, Self.isWebsiteURL(url) { webView.load(action.request) }
        return nil
    }
    func webViewDidClose(_ webView: WKWebView) { exportPopups.removeValue(forKey: ObjectIdentifier(webView)) }

    func webView(_ webView: WKWebView, runOpenPanelWith parameters: WKOpenPanelParameters, initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping ([URL]?) -> Void) {
        guard let window = view.window else { completionHandler(nil); return }
        let panel = NSOpenPanel(); panel.canChooseDirectories = parameters.allowsDirectories
        panel.canChooseFiles = true; panel.allowsMultipleSelection = parameters.allowsMultipleSelection
        panel.beginSheetModal(for: window) { response in completionHandler(response == .OK ? panel.urls : nil) }
    }
    func webView(_ webView: WKWebView, runJavaScriptAlertPanelWithMessage message: String, initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping () -> Void) {
        guard let window = view.window else { completionHandler(); return }
        let alert = NSAlert(); alert.messageText = "内置无限画布"; alert.informativeText = message; alert.addButton(withTitle: "确定")
        alert.beginSheetModal(for: window) { _ in completionHandler() }
    }
    func webView(_ webView: WKWebView, runJavaScriptConfirmPanelWithMessage message: String, initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping (Bool) -> Void) {
        guard let window = view.window else { completionHandler(false); return }
        let alert = NSAlert(); alert.messageText = "内置无限画布确认"; alert.informativeText = message
        alert.addButton(withTitle: "确定"); alert.addButton(withTitle: "取消")
        alert.beginSheetModal(for: window) { completionHandler($0 == .alertFirstButtonReturn) }
    }
    func webView(_ webView: WKWebView, runJavaScriptTextInputPanelWithPrompt prompt: String, defaultText: String?, initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping (String?) -> Void) {
        guard let window = view.window else { completionHandler(nil); return }
        let alert = NSAlert(); alert.messageText = "内置无限画布"; alert.informativeText = prompt
        let input = NSTextField(string: defaultText ?? ""); input.frame = NSRect(x: 0, y: 0, width: 320, height: 24); alert.accessoryView = input
        alert.addButton(withTitle: "确定"); alert.addButton(withTitle: "取消")
        alert.beginSheetModal(for: window) { completionHandler($0 == .alertFirstButtonReturn ? input.stringValue : nil) }
    }
    func webView(_ webView: WKWebView, navigationAction: WKNavigationAction, didBecome download: WKDownload) { retain(download); exportPopups.removeValue(forKey: ObjectIdentifier(webView)) }
    func webView(_ webView: WKWebView, navigationResponse: WKNavigationResponse, didBecome download: WKDownload) { retain(download); exportPopups.removeValue(forKey: ObjectIdentifier(webView)) }
    private func retain(_ download: WKDownload) { downloads[ObjectIdentifier(download)] = download; download.delegate = self }
    func download(_ download: WKDownload, decideDestinationUsing response: URLResponse, suggestedFilename: String, completionHandler: @escaping (URL?) -> Void) {
        guard let window = view.window else { completionHandler(nil); return }
        let panel = NSSavePanel(); panel.nameFieldStringValue = (suggestedFilename as NSString).lastPathComponent
        panel.beginSheetModal(for: window) { completionHandler($0 == .OK ? panel.url : nil) }
    }
    func downloadDidFinish(_ download: WKDownload) { downloads.removeValue(forKey: ObjectIdentifier(download)) }
    func download(_ download: WKDownload, didFailWithError error: Error, resumeData: Data?) { downloads.removeValue(forKey: ObjectIdentifier(download)); show(error) }
}
