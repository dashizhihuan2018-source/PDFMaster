import AppKit
import WebKit

/// Hosts the original website, not a substitute whiteboard or an iframe.
/// WebKit owns this app's cookies/IndexedDB; no PDF or native account bridge is installed.
@MainActor
final class CanvasWebsiteController: NSViewController, WKNavigationDelegate, WKUIDelegate, WKDownloadDelegate {
    static let startURL = URL(string: "https://infinitecanvas.maip.top/canvas?mode=recent")!
    private(set) var webView: WKWebView!
    private let status = NSTextField(labelWithString: "正在加载无限画布完整网站…")
    private var downloads: [ObjectIdentifier: WKDownload] = [:]
    private var exportPopups: [ObjectIdentifier: WKWebView] = [:]

    override func loadView() {
        let root = NSView(); root.wantsLayer = true
        root.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        webView = WKWebView(frame: .zero, configuration: config)
        webView.navigationDelegate = self; webView.uiDelegate = self
        webView.allowsBackForwardNavigationGestures = true
        webView.setAccessibilityLabel("无限画布完整网站工作区")
        status.font = .systemFont(ofSize: 11); status.textColor = .secondaryLabelColor
        status.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let bar = NSStackView(); bar.orientation = .horizontal; bar.spacing = 12
        bar.edgeInsets = NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 10)
        let reload = NSButton(title: "重新加载", target: self, action: #selector(reloadWebsite))
        let browser = NSButton(title: "在浏览器打开", target: self, action: #selector(openInBrowser))
        for button in [reload, browser] { button.bezelStyle = .inline; button.font = .systemFont(ofSize: 11) }
        for child in [status, NSView(), reload, browser] { bar.addArrangedSubview(child) }
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
        url.scheme == "https" && url.host == startURL.host && (url.port == nil || url.port == 443) && url.user == nil && url.password == nil
    }
    @objc private func reloadWebsite() {
        status.stringValue = "正在加载无限画布完整网站…"
        if let url = webView.url, Self.isWebsiteURL(url) { webView.reload() }
        else { webView.load(URLRequest(url: Self.startURL)) }
    }
    @objc private func openInBrowser() { NSWorkspace.shared.open(webView.url.flatMap { Self.isWebsiteURL($0) ? $0 : nil } ?? Self.startURL) }

    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = action.request.url else { decisionHandler(.cancel); return }
        if action.shouldPerformDownload { decisionHandler(.download); return }
        // FileSaver's macOS WebView fallback writes a data URL into its blank popup.
        if exportPopups[ObjectIdentifier(webView)] != nil, ["data", "blob"].contains(url.scheme ?? "") {
            decisionHandler(.download); return
        }
        // Subframe/resource requests remain under WebKit's origin policy.
        if action.targetFrame?.isMainFrame == false || Self.isWebsiteURL(url) || url.absoluteString == "about:blank" {
            decisionHandler(.allow); return
        }
        decisionHandler(.cancel)
        if action.navigationType == .linkActivated, ["https", "mailto"].contains(url.scheme ?? "") { NSWorkspace.shared.open(url) }
    }
    func webView(_ webView: WKWebView, decidePolicyFor response: WKNavigationResponse, decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void) {
        decisionHandler(response.canShowMIMEType ? .allow : .download)
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        status.stringValue = "无限画布 · 在线完整界面 · 网站登录独立于本机账户"
    }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { show(error) }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { show(error) }
    private func show(_ error: Error) {
        guard (error as NSError).code != NSURLErrorCancelled else { return }
        status.stringValue = "网站加载失败：\(error.localizedDescription) · 可重新加载或在浏览器打开"
    }
    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) { status.stringValue = "网站进程已停止，请点击重新加载恢复工作区。" }
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for action: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        if action.request.url?.absoluteString.isEmpty != false || action.request.url?.absoluteString == "about:blank" {
            let popup = WKWebView(frame: .zero, configuration: configuration)
            popup.navigationDelegate = self; popup.uiDelegate = self
            exportPopups[ObjectIdentifier(popup)] = popup
            return popup
        }
        if let url = action.request.url, Self.isWebsiteURL(url) { webView.load(action.request) }
        else if let url = action.request.url, ["https", "mailto"].contains(url.scheme ?? "") { NSWorkspace.shared.open(url) }
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
        let alert = NSAlert(); alert.messageText = "无限画布网站"; alert.informativeText = message; alert.addButton(withTitle: "确定")
        alert.beginSheetModal(for: window) { _ in completionHandler() }
    }
    func webView(_ webView: WKWebView, runJavaScriptConfirmPanelWithMessage message: String, initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping (Bool) -> Void) {
        guard let window = view.window else { completionHandler(false); return }
        let alert = NSAlert(); alert.messageText = "无限画布网站确认"; alert.informativeText = message
        alert.addButton(withTitle: "确定"); alert.addButton(withTitle: "取消")
        alert.beginSheetModal(for: window) { completionHandler($0 == .alertFirstButtonReturn) }
    }
    func webView(_ webView: WKWebView, runJavaScriptTextInputPanelWithPrompt prompt: String, defaultText: String?, initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping (String?) -> Void) {
        guard let window = view.window else { completionHandler(nil); return }
        let alert = NSAlert(); alert.messageText = "无限画布网站"; alert.informativeText = prompt
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
