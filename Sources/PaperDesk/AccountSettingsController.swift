import AppKit
import PaperCore

@MainActor
final class AccountSettingsController: NSViewController, NSWindowDelegate {
    let usernameField = NSTextField(string: "")
    let passwordField = NSSecureTextField(string: "")
    let keyField = NSSecureTextField(string: "")
    let agnesKeyField = NSSecureTextField(string: "")
    let deepSeekKeyField = NSSecureTextField(string: "")
    let oldPasswordField = NSSecureTextField(string: "")
    let newPasswordField = NSSecureTextField(string: "")
    let confirmPasswordField = NSSecureTextField(string: "")
    let rememberButton = NSButton(checkboxWithTitle: "保持本机登录（仅此 Mac）", target: nil, action: nil)
    let loginButton = NSButton(title: "登录", target: nil, action: nil)
    let registerButton = NSButton(title: "创建本机账户", target: nil, action: nil)
    let logoutButton = NSButton(title: "退出登录", target: nil, action: nil)
    let saveKeyButton = NSButton(title: "保存 Key 到钥匙串", target: nil, action: nil)
    let removeKeyButton = NSButton(title: "移除已存 Key", target: nil, action: nil)
    let saveAgnesKeyButton = NSButton(title: "保存 Agnes Key 到钥匙串", target: nil, action: nil)
    let removeAgnesKeyButton = NSButton(title: "移除 Agnes Key", target: nil, action: nil)
    let saveDeepSeekKeyButton = NSButton(title: "保存 DeepSeek Key 到钥匙串", target: nil, action: nil)
    let removeDeepSeekKeyButton = NSButton(title: "移除 DeepSeek Key", target: nil, action: nil)
    let changePasswordButton = NSButton(title: "修改密码", target: nil, action: nil)
    let securityNotice = NSTextField(wrappingLabelWithString: "")
    let keyStatus = NSTextField(wrappingLabelWithString: "")
    let agnesKeyStatus = NSTextField(wrappingLabelWithString: "")
    let deepSeekKeyStatus = NSTextField(wrappingLabelWithString: "")
    let accountStatus = NSTextField(wrappingLabelWithString: "")
    let statusLabel = NSTextField(wrappingLabelWithString: "")
    private let store: AccountStore
    private var settingsWindow: NSWindow?
    private var lastUsername: String?

    convenience init() { self.init(store: .shared) }
    init(store: AccountStore) { self.store = store; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 550, height: 700))
        securityNotice.stringValue = "账户、登录记忆与 Key 仅保存在本机；不是云端登录，也不提供系统管理员权限。初始账户 Nankong 使用公开默认密码，请登录后立即修改。这里的记忆仅指登录与 Key，不是 AI 长期对话记忆。"
        statusLabel.textColor = .secondaryLabelColor
        let title = NSTextField(labelWithString: "本机账户与 AI 设置")
        title.font = .systemFont(ofSize: 21, weight: .semibold)
        for (field, name) in [(usernameField, "本机用户名"), (passwordField, "本机登录密码"), (keyField, "DMX API Key"), (agnesKeyField, "Agnes API Key"), (deepSeekKeyField, "DeepSeek API Key"),
                              (oldPasswordField, "当前密码"), (newPasswordField, "新密码"), (confirmPasswordField, "确认新密码")] {
            field.placeholderString = name; field.setAccessibilityLabel(name)
            field.widthAnchor.constraint(greaterThanOrEqualToConstant: 450).isActive = true
        }
        let content = NSStackView(views: [title, securityNotice, accountStatus, usernameField, passwordField,
            rememberButton, row(loginButton, registerButton, logoutButton), section("DMX Key · 当前账户"), keyStatus,
            keyField, row(saveKeyButton, removeKeyButton), section("DeepSeek Key · 当前账户（PDF 文字问答）"), deepSeekKeyStatus,
            deepSeekKeyField, row(saveDeepSeekKeyButton, removeDeepSeekKeyButton), section("Agnes Key · 当前账户"), agnesKeyStatus,
            agnesKeyField, row(saveAgnesKeyButton, removeAgnesKeyButton), section("修改本机密码"), oldPasswordField,
            newPasswordField, confirmPasswordField, changePasswordButton, statusLabel])
        content.orientation = .vertical; content.alignment = .leading; content.spacing = 10
        content.translatesAutoresizingMaskIntoConstraints = false
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true; scroll.autohidesScrollers = true; scroll.drawsBackground = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        let document = AccountSettingsDocumentView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(content); scroll.documentView = document; view.addSubview(scroll)
        NSLayoutConstraint.activate([scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor), scroll.topAnchor.constraint(equalTo: view.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            document.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor),
            content.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 28),
            content.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -28),
            content.topAnchor.constraint(equalTo: document.topAnchor, constant: 24),
            content.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -24)])
        for label in [securityNotice, keyStatus, agnesKeyStatus, deepSeekKeyStatus, accountStatus, statusLabel] {
            label.widthAnchor.constraint(equalTo: content.widthAnchor).isActive = true
        }
        for (button, action) in [(loginButton, #selector(performLogin)), (registerButton, #selector(performRegister)),
            (logoutButton, #selector(performLogout)), (saveKeyButton, #selector(performSaveKey)),
            (removeKeyButton, #selector(performRemoveKey)), (saveAgnesKeyButton, #selector(performSaveAgnesKey)),
            (removeAgnesKeyButton, #selector(performRemoveAgnesKey)), (saveDeepSeekKeyButton, #selector(performSaveDeepSeekKey)),
            (removeDeepSeekKeyButton, #selector(performRemoveDeepSeekKey)), (changePasswordButton, #selector(performChangePassword))] {
            button.target = self; button.action = action; button.bezelStyle = .rounded
        }
        usernameField.stringValue = store.rememberedUsername ?? "Nankong"
        NotificationCenter.default.addObserver(self, selector: #selector(accountChanged), name: AccountStore.didChangeNotification, object: store)
        refresh()
    }
    private func section(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text); label.font = .systemFont(ofSize: 13, weight: .semibold); return label
    }
    private func row(_ buttons: NSButton...) -> NSStackView {
        let row = NSStackView(views: buttons); row.orientation = .horizontal; row.spacing = 10; return row
    }
    func showWindow(relativeTo parent: NSWindow? = nil) {
        if let settingsWindow { settingsWindow.makeKeyAndOrderFront(nil); return }
        let availableHeight = (parent?.screen ?? NSScreen.main)?.visibleFrame.height ?? 800
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 550, height: min(750, max(400, availableHeight - 80))),
                              styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.title = "PDF Master · 本机账户与 AI 设置"
        window.contentViewController = self; window.isReleasedWhenClosed = false; window.delegate = self
        settingsWindow = window
        if let parent {
            window.setFrameOrigin(NSPoint(x: parent.frame.midX - window.frame.width / 2,
                                          y: parent.frame.midY - window.frame.height / 2))
        } else { window.center() }
        window.setFrame(window.constrainFrameRect(window.frame, to: parent?.screen ?? NSScreen.main), display: false)
        window.makeKeyAndOrderFront(nil)
    }
    func windowWillClose(_ notification: Notification) { clearSecureFields(); settingsWindow = nil }

    @objc func performLogin() {
        do {
            try store.login(username: usernameField.stringValue, password: passwordField.stringValue, remember: rememberButton.state == .on)
            statusLabel.stringValue = store.isUsingDefaultPassword ? "已登录。请立即修改公开默认密码。" : "已登录本机账户。"
        } catch { showFailure(error) }
        passwordField.stringValue = ""; refresh()
    }
    @objc func performRegister() {
        do {
            try store.register(username: usernameField.stringValue, password: passwordField.stringValue)
            statusLabel.stringValue = "已创建本机账户，请使用刚设置的密码登录。"
        } catch { showFailure(error) }
        passwordField.stringValue = ""; refresh()
    }
    @objc func performLogout() {
        store.logout(); clearSecureFields(); refresh()
        if let error = store.lastOperationError { showFailure(error) }
        else { statusLabel.stringValue = "已退出登录；钥匙串中的账户 Key 仍保留，需再次登录才能使用。" }
    }
    @objc func performSaveKey() {
        do { try store.saveDMXKey(keyField.stringValue); statusLabel.stringValue = "Key 已安全保存在当前账户的 macOS 钥匙串中。" }
        catch { showFailure(error) }
        keyField.stringValue = ""; refresh()
    }
    @objc func performRemoveKey() {
        do { try store.deleteDMXKey(); statusLabel.stringValue = "已移除当前账户的 DMX Key。" }
        catch { showFailure(error) }
        keyField.stringValue = ""; refresh()
    }
    @objc func performSaveAgnesKey() {
        do { try store.saveAgnesKey(agnesKeyField.stringValue); statusLabel.stringValue = "Agnes Key 已安全保存在当前账户的 macOS 钥匙串中。" }
        catch { showFailure(error) }
        agnesKeyField.stringValue = ""; refresh()
    }
    @objc func performRemoveAgnesKey() {
        do { try store.deleteAgnesKey(); statusLabel.stringValue = "已移除当前账户的 Agnes Key。" }
        catch { showFailure(error) }
        agnesKeyField.stringValue = ""; refresh()
    }
    @objc func performSaveDeepSeekKey() {
        do { try store.saveDeepSeekKey(deepSeekKeyField.stringValue); statusLabel.stringValue = "DeepSeek Key 已安全保存在当前账户的 macOS 钥匙串中。" }
        catch { showFailure(error) }
        deepSeekKeyField.stringValue = ""; refresh()
    }
    @objc func performRemoveDeepSeekKey() {
        do { try store.deleteDeepSeekKey(); statusLabel.stringValue = "已移除当前账户的 DeepSeek Key。" }
        catch { showFailure(error) }
        deepSeekKeyField.stringValue = ""; refresh()
    }
    @objc func performChangePassword() {
        guard newPasswordField.stringValue == confirmPasswordField.stringValue else {
            statusLabel.stringValue = "两次新密码不一致，请重新确认。"; return
        }
        do {
            try store.changePassword(old: oldPasswordField.stringValue, new: newPasswordField.stringValue)
            clearSecureFields(); statusLabel.stringValue = "本机密码已修改。"
        } catch { showFailure(error) }
        refresh()
    }
    @objc private func accountChanged() {
        if lastUsername != store.currentUsername { clearSecureFields() }
        refresh()
    }
    private func clearSecureFields() {
        view.window?.endEditing(for: nil)
        for field in [passwordField, keyField, agnesKeyField, deepSeekKeyField, oldPasswordField, newPasswordField, confirmPasswordField] { field.stringValue = "" }
    }
    private func showFailure(_ error: Error) {
        // Never surface arbitrary upstream strings that could contain credentials.
        statusLabel.stringValue = (error as? AccountStoreFailure)?.errorDescription ?? "本机设置暂不可用，请稍后再试。"
    }
    private func refresh() {
        guard isViewLoaded else { return }
        lastUsername = store.currentUsername
        let loggedIn = store.isLoggedIn
        let healthy = store.initializationError == nil
        usernameField.isEnabled = !loggedIn && healthy; passwordField.isEnabled = !loggedIn && healthy
        rememberButton.isEnabled = !loggedIn && healthy
        if loggedIn { rememberButton.state = store.keepsLogin ? .on : .off }
        loginButton.isEnabled = !loggedIn && healthy; registerButton.isEnabled = !loggedIn && healthy
        logoutButton.isEnabled = loggedIn
        for control in [keyField, agnesKeyField, deepSeekKeyField, oldPasswordField, newPasswordField, confirmPasswordField] { control.isEnabled = loggedIn }
        for button in [saveKeyButton, removeKeyButton, saveAgnesKeyButton, removeAgnesKeyButton, saveDeepSeekKeyButton, removeDeepSeekKeyButton, changePasswordButton] { button.isEnabled = loggedIn }
        accountStatus.stringValue = store.currentUsername.map { "当前本机账户：\($0)" } ?? "尚未登录本机账户"
        for (provider, label) in [(PDFTextProvider.dmx, keyStatus), (.agnes, agnesKeyStatus), (.deepseek, deepSeekKeyStatus)] {
            if loggedIn {
                do { label.stringValue = try store.hasSavedKey(for: provider) ? "已保存 \(provider.title) Key（已隐藏）；与其他服务商和账户独立保存。" : "未保存 \(provider.title) Key；请选择服务商并绑定你自己的 Key，确认发送后才使用。" }
                catch { label.stringValue = "无法确认 \(provider.title) Key 状态；没有读取或回显密钥。" }
            } else { label.stringValue = "请先登录。\(provider.title) Key 仅保存到当前账户，不读取其他账户或服务商的 Key。" }
        }
        if let error = store.initializationError { showFailure(error) }
    }
}

private final class AccountSettingsDocumentView: NSView {
    override var isFlipped: Bool { true }
}
