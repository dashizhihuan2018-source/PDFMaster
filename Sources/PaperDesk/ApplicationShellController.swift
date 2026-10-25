import AppKit
import PDFKit
import PaperCore

@MainActor
final class CanvasTab: Identifiable {
    let id = UUID()
    let controller: CanvasController
    var title: String { controller.session.board.title + (controller.session.isDirty ? " •" : "") }
    init(controller: CanvasController) { self.controller = controller }
}

@MainActor
final class DocumentTab: Identifiable {
    let id = UUID()
    let controller: WorkbenchController
    var sourceURL: URL? { controller.workspace.sourceURL }
    let generatedTitle: String?
    var title: String { sourceURL?.lastPathComponent ?? generatedTitle ?? "未命名 PDF" }
    init(controller: WorkbenchController, sourceURL: URL?, generatedTitle: String? = nil) {
        self.controller = controller; self.generatedTitle = generatedTitle
    }
}

@MainActor
final class ApplicationShellController: NSViewController {
    let recentStore: RecentDocumentStore
    let preferredTools: PreferredToolStore
    private(set) var documentTabs: [DocumentTab] = []
    private(set) var canvasTabs: [CanvasTab] = []
    private(set) var activeTabID: UUID?
    private var fileImportCancellation: LocalToolCancellation?
    var isFileImportRunning: Bool { fileImportCancellation != nil }
    var onFileImportStateChange: ((Bool) -> Void)?
    func cancelFileImport() { fileImportCancellation?.cancel() }
    private var lastActiveTabID: UUID?
    private let tabBar = NSStackView()
    private let windowControls = NSStackView()
    private var homeTab: NSButton?
    private var newTabButton: NSButton?
    var chromeHeight: CGFloat { 40 }
    var homeTabIsIconOnly: Bool { homeTab?.title.isEmpty == true && homeTab?.image != nil }
    var newTabIsBorderless: Bool { newTabButton?.isBordered == false }
    var nativeWindowControlCount: Int { windowControls.arrangedSubviews.count }
    func activateNewTabForTesting() { newTabButton?.performClick(nil) }
    var highlightedDocumentTabID: UUID? {
        tabBar.arrangedSubviews.compactMap { $0 as? NSStackView }.compactMap { $0.arrangedSubviews.first as? TabButton }.first { $0.state == .on }?.tabID
    }
    private let content = NSView()
    private lazy var dashboard = DashboardViewController(shell: self)
    var dashboardController: DashboardViewController { dashboard }
    var activeWorkbench: WorkbenchController? { documentTabs.first { $0.id == activeTabID }?.controller }
    var activeCanvas: CanvasController? { canvasTabs.first { $0.id == activeTabID }?.controller }
    var onActiveWorkbenchChange: ((WorkbenchController?) -> Void)?
    private(set) var lastOpenError: Error?
    private(set) var imageImportController: ImageImportController?
    /// Injectable native presentation boundary; normal app uses its attached window.
    var imageImportPresenter: ((ImageImportController) -> Void)?
    private(set) var pdfImageExportController: PDFImageExportController?
    private let injectedAccountStore: AccountStore?
    private let aiPreferences: UserDefaults
    private let aiClient: DMXAIClient
    private var accountEpoch = 0
    lazy var accountStore: AccountStore = {
        let store = injectedAccountStore ?? .shared
        NotificationCenter.default.addObserver(self, selector: #selector(accountChanged), name: AccountStore.didChangeNotification, object: store)
        return store
    }()
    private(set) var accountSettingsController: AccountSettingsController?
    private(set) var aiStudioController: AIStudioController?
    private(set) var copilotController: PDFCopilotController?
    private weak var copilotWorkbench: WorkbenchController?
    private var studios: [AIStudioController] = []
    var accountSettingsPresenter: ((AccountSettingsController) -> Void)?
    var aiStudioPresenter: ((AIStudioController) -> Void)?
    var copilotPresenter: ((PDFCopilotController) -> Void)?
    private var aiIdentity: String {
        "\(accountEpoch):\(accountStore.currentUsername ?? "guest"):\(activeTabID?.uuidString ?? "home")"
    }
    /// Injectable native presentation boundary; normal app uses its attached window.
    var pdfImageExportPresenter: ((PDFImageExportController) -> Void)?

    init(recentStore: RecentDocumentStore, preferredTools: PreferredToolStore? = nil,
         accountStore: AccountStore? = nil, aiPreferences: UserDefaults = .standard, aiClient: DMXAIClient = DMXAIClient()) {
        self.recentStore = recentStore
        self.injectedAccountStore = accountStore; self.aiPreferences = aiPreferences; self.aiClient = aiClient
        let defaultURL = recentStore.url.deletingLastPathComponent().appendingPathComponent("preferred-tools.json")
        self.preferredTools = preferredTools ?? PreferredToolStore(url: defaultURL)
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    @objc private func accountChanged() {
        accountEpoch += 1
        invalidateAIContext()
        if isViewLoaded, activeTabID == nil { dashboard.refreshContents() }
    }
    private func invalidateAIContext() {
        hideCopilot()
        studios.forEach { $0.invalidateContext() }
    }
    func showAccountSettings() {
        let controller = accountSettingsController ?? AccountSettingsController(store: accountStore)
        accountSettingsController = controller
        if let accountSettingsPresenter { accountSettingsPresenter(controller) }
        else { controller.showWindow(relativeTo: view.window) }
    }
    @discardableResult func showAIStudio(for canvas: CanvasController) -> AIStudioController? {
        guard activeCanvas === canvas else { return nil }
        let identity = aiIdentity
        let username = accountStore.currentUsername
        let taskKey = username.map { "PDFMaster.VideoTask." + $0.lowercased() }
        let preferences = aiPreferences
        let controller = AIStudioController(keyProvider: { [weak self] in
            guard let self, self.aiIdentity == identity else { return nil }
            return try self.accountStore.readDMXKey()
        }, onImageGenerated: { [weak self, weak canvas] data, name in
            guard let self, let canvas, self.aiIdentity == identity, self.activeCanvas === canvas else { return }
            do { try canvas.importAsset(data: data, name: name, kind: .image) }
            catch { self.lastOpenError = error }
        }, restoredVideoTaskID: taskKey.flatMap { preferences.string(forKey: $0) }, onVideoTaskSaved: { id in
            guard let taskKey else { return }
            preferences.set(id, forKey: taskKey)
        }, sessionIdentityProvider: { [weak self] in self?.aiIdentity }, client: aiClient)
        controller.onOpenSettings = { [weak self] in self?.showAccountSettings() }
        studios.removeAll { !$0.isRunning && $0.window?.isVisible != true }
        studios.append(controller); aiStudioController = controller
        if let aiStudioPresenter { aiStudioPresenter(controller) }
        else { controller.showWindow(relativeTo: view.window) }
        return controller
    }
    @discardableResult func showCopilot() -> Bool {
        guard let workbench = activeWorkbench else { return false }
        if copilotWorkbench === workbench, copilotController != nil { hideCopilot(); return true }
        hideCopilot()
        let controller = PDFCopilotController(documentProvider: { [weak self, weak workbench] in
            guard self?.activeWorkbench === workbench else { return nil }
            return workbench?.workspace.document
        }, currentPageProvider: { [weak workbench] in workbench?.copilotPageIndex ?? 0 }, keyProvider: { [weak self] in
            try self?.accountStore.readDMXKey()
        }, onNavigate: { [weak self, weak workbench] index in
            guard self?.activeWorkbench === workbench else { return }
            workbench?.navigateCopilot(to: index)
        }, revisionProvider: { [weak workbench] in workbench?.workspace.revision ?? -1 },
        sessionIdentityProvider: { [weak self] in self?.aiIdentity }, client: aiClient,
        agnesKeyProvider: { [weak self] in try self?.accountStore.readAgnesKey() }, initialProvider: .agnes,
        deepSeekKeyProvider: { [weak self] in try self?.accountStore.readDeepSeekKey() })
        controller.onOpenSettings = { [weak self] in self?.showAccountSettings() }
        controller.onClose = { [weak self] in self?.hideCopilot() }
        copilotController = controller; copilotWorkbench = workbench
        workbench.setCopilotPanel(controller.panelView)
        if let copilotPresenter { copilotPresenter(controller) }
        return true
    }
    func hideCopilot() {
        copilotController?.close(); copilotWorkbench?.setCopilotPanel(nil)
        copilotController = nil; copilotWorkbench = nil
    }

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 1280, height: 820)); root.wantsLayer = true; root.layer?.backgroundColor = ShellPalette.canvas.cgColor
        tabBar.orientation = .horizontal; tabBar.distribution = .fill; tabBar.spacing = 0; tabBar.alignment = .centerY; tabBar.edgeInsets = NSEdgeInsets(top: 0, left: 100, bottom: 0, right: 8)
        tabBar.wantsLayer = true; tabBar.layer?.backgroundColor = ShellPalette.top.cgColor
        windowControls.orientation = .horizontal; windowControls.spacing = 8; windowControls.alignment = .centerY
        [tabBar, content].forEach { $0.translatesAutoresizingMaskIntoConstraints = false; root.addSubview($0) }
        NSLayoutConstraint.activate([
            tabBar.topAnchor.constraint(equalTo: root.topAnchor), tabBar.leadingAnchor.constraint(equalTo: root.leadingAnchor), tabBar.trailingAnchor.constraint(equalTo: root.trailingAnchor), tabBar.heightAnchor.constraint(equalToConstant: 40),
            content.topAnchor.constraint(equalTo: tabBar.bottomAnchor), content.leadingAnchor.constraint(equalTo: root.leadingAnchor), content.trailingAnchor.constraint(equalTo: root.trailingAnchor), content.bottomAnchor.constraint(equalTo: root.bottomAnchor)
        ])
        windowControls.translatesAutoresizingMaskIntoConstraints = false; root.addSubview(windowControls)
        NSLayoutConstraint.activate([windowControls.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 12), windowControls.centerYAnchor.constraint(equalTo: tabBar.centerYAnchor), windowControls.heightAnchor.constraint(equalToConstant: 20)])
        view = root; showHome()
    }

    /// Uses AppKit's actual standard buttons, preserving their window actions.
    func installWindowChrome(in window: NSWindow) {
        _ = view
        guard windowControls.arrangedSubviews.isEmpty else { return }
        let buttons = [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton].compactMap { window.standardWindowButton($0) }
        for button in buttons {
            button.removeFromSuperview(); button.translatesAutoresizingMaskIntoConstraints = false
            windowControls.addArrangedSubview(button)
            button.widthAnchor.constraint(equalToConstant: 14).isActive = true
            button.heightAnchor.constraint(equalToConstant: 14).isActive = true
        }
    }

    @discardableResult func open(url: URL, passwordPrompt: () -> String?) -> Bool {
        lastOpenError = nil
        guard url.isFileURL else { lastOpenError = CocoaError(.fileReadUnsupportedScheme); return false }
        if url.pathExtension.lowercased() == "pdfmasterboard" { return openCanvas(url: url) }
        let normalized = RecentDocumentStore.normalize(url)
        if let existing = documentTabs.first(where: { $0.sourceURL == normalized }) { activate(tabID: existing.id); return true }
        let controller = WorkbenchController()
        do {
            let admitted = try DocumentAdmission.admit(url: normalized, passwordPrompt: passwordPrompt, shouldReplace: { true }) { password in
                if let password { try controller.workspace.open(normalized, password: password) } else { try controller.workspace.open(normalized) }
            }
            guard admitted else { return false }
            _ = controller.view
            controller.captureAdmittedSource(normalized)
            controller.resetDocumentInteractionState(); controller.refreshDocument()
            let tab = DocumentTab(controller: controller, sourceURL: normalized); documentTabs.append(tab)
            do { try recentStore.recordSuccessfulOpen(normalized) } catch { controller.setInteractionStatus("已打开文档；最近记录保存失败：\(error.localizedDescription)") }
            activate(tabID: tab.id); return true
        } catch { lastOpenError = error; return false }
    }

    @discardableResult func open(url: URL) -> Bool { open(url: url) { DocumentToolPanels.passwordForOpening() } }
    /// Home drops create a new merged tab. Never append to a hidden document.
    @discardableResult func importDroppedFiles(_ urls: [URL]) async throws -> DocumentTab {
        guard fileImportCancellation == nil else { throw FilePDFImportError.failed("导入", "另一个文件导入仍在运行") }
        let flag = LocalToolCancellation(); fileImportCancellation = flag
        onFileImportStateChange?(true)
        let activity = LocalToolActivity.register(flag)
        defer { fileImportCancellation = nil; LocalToolActivity.finish(activity); onFileImportStateChange?(false) }
        let batch = try await FileImportJob.prepare(urls, cancellation: flag)
        guard !flag.isCancelled, !batch.isEmpty else { throw FilePDFImportError.cancelled }
        let controller = WorkbenchController()
        try controller.workspace.createUnsavedFileMerge(batch)
        _ = controller.view
        controller.resetDocumentInteractionState(); controller.refreshDocument()
        controller.setInteractionStatus("\(urls.count) 个文件已合并为独立 PDF · 来源未修改 · 请选择保存位置")
        let name = urls.count == 1 ? urls[0].deletingPathExtension().lastPathComponent + ".pdf" : "文件合并.pdf"
        let tab = DocumentTab(controller: controller, sourceURL: nil, generatedTitle: name)
        documentTabs.append(tab); activate(tabID: tab.id); return tab
    }
    @discardableResult func createBlankDocument() -> DocumentTab {
        let controller = WorkbenchController(); _ = controller.view
        controller.perform { try controller.workspace.createBlank(); controller.refreshDocument() }
        let tab = DocumentTab(controller: controller, sourceURL: nil); documentTabs.append(tab); activate(tabID: tab.id); return tab
    }
    @discardableResult func createCanvas(board: CanvasBoard = CanvasBoard(title: "未命名无边记")) -> CanvasTab {
        let controller = CanvasController(session: CanvasSession(board: board))
        let tab = CanvasTab(controller: controller)
        controller.onShowAI = { [weak self, weak controller] in
            guard let controller else { return }
            _ = self?.showAIStudio(for: controller)
        }
        controller.onTitleChange = { [weak self] in self?.rebuildTabs() }
        controller.onOpenPDF = { [weak self] data, name in
            do { try self?.openCanvasPDF(data: data, name: name) }
            catch {
                self?.lastOpenError = error
                let alert = NSAlert(); alert.messageText = "无法打开画布中的 PDF"; alert.informativeText = error.localizedDescription
                alert.runModal()
            }
        }
        canvasTabs.append(tab); activate(tabID: tab.id)
        return tab
    }
    @discardableResult func openCanvas(url: URL) -> Bool {
        lastOpenError = nil
        guard url.isFileURL else { lastOpenError = CocoaError(.fileReadUnsupportedScheme); return false }
        let normalized = RecentDocumentStore.normalize(url)
        if let tab = canvasTabs.first(where: { $0.controller.sourceURL == normalized }) {
            activate(tabID: tab.id); return true
        }
        do {
            let board = try CanvasController.validatedAssets(CanvasBoard.load(from: normalized))
            let tab = createCanvas(board: board); tab.controller.sourceURL = normalized
            return true
        } catch { lastOpenError = error; return false }
    }
    /// An embedded PDF is a new, independent editor copy, never a write-through to its card.
    @discardableResult func openCanvasPDF(data: Data, name: String) throws -> DocumentTab {
        guard data.count <= 50 * 1024 * 1024, let document = PDFDocument(data: data),
              !document.isEncrypted, document.pageCount > 0,
              !DocumentAdmission.hasInteractiveWidgets(document) else { throw CocoaError(.fileReadCorruptFile) }
        let controller = WorkbenchController()
        try controller.workspace.createUnsavedPDFCopy(data: data)
        _ = controller.view
        controller.resetDocumentInteractionState(); controller.refreshDocument()
        controller.setInteractionStatus("来自无边记的独立 PDF 副本 · 编辑不会修改画布卡片或原文件")
        let tab = DocumentTab(controller: controller, sourceURL: nil, generatedTitle: URL(fileURLWithPath: name).lastPathComponent)
        documentTabs.append(tab); activate(tabID: tab.id); return tab
    }
    /// Explicit document-to-board bridge; editing rights never imply bypassing copy restrictions.
    @discardableResult func sendActivePDFToCanvas() throws -> CanvasTab {
        guard let workbench = activeWorkbench, workbench.workspace.allowsCopying, !workbench.workspace.isEncryptedSource,
              let document = workbench.workspace.document, !document.isEncrypted,
              !DocumentAdmission.hasInteractiveWidgets(document), let data = document.dataRepresentation(),
              data.count <= 50 * 1024 * 1024 else { throw CocoaError(.fileReadNoPermission) }
        let name = documentTabs.first { $0.id == activeTabID }?.title ?? "文档.pdf"
        let admission = PDFWorkspace()
        try admission.createUnsavedPDFCopy(data: data)
        let board = try CanvasBoard(title: "\(name) · 画布", elements: [CanvasElement(kind: .pdf, frame: CGRect(x: 80, y: 80, width: 260, height: 340), assetData: data, assetName: name)]).validated()
        let tab = createCanvas()
        try tab.controller.session.mutate { $0 = board }
        return tab
    }
    @discardableResult func createImportedDocument(_ output: ImageImportOutput) throws -> DocumentTab {
        guard let check = PDFDocument(data: output.data), check.pageCount == output.pageCount,
              output.pageCount > 0, !DocumentAdmission.hasInteractiveWidgets(check) else {
            throw ImageImportControllerFailure.invalidOutput
        }
        let controller = WorkbenchController()
        try controller.workspace.createUnsavedDocument(data: output.data)
        _ = controller.view
        controller.resetDocumentInteractionState(); controller.refreshDocument()
        controller.setInteractionStatus("图片已创建独立 PDF · 来源图片未修改 · 请选择保存位置")
        let tab = DocumentTab(controller: controller, sourceURL: nil,
            generatedTitle: URL(fileURLWithPath: output.suggestedName).lastPathComponent)
        documentTabs.append(tab); activate(tabID: tab.id)
        return tab
    }
    private func startImageImport() -> Bool {
        guard imageImportController?.isRunning != true else { return false }
        let window = view.window
        guard imageImportPresenter != nil || (window != nil && window?.attachedSheet == nil) else { return false }
        imageImportController?.invalidate()
        let importer = ImageImportController()
        imageImportController = importer
        importer.onImport = { [weak self, weak importer] output in
            guard let self, let importer, self.imageImportController === importer else { return }
            do {
                _ = try self.createImportedDocument(output)
                self.imageImportController = nil
            } catch { self.lastOpenError = error; self.activeWorkbench?.setInteractionStatus(ImageImportController.message(for: error)) }
        }
        importer.onFailure = { [weak self, weak importer] error in
            guard let self, let importer, self.imageImportController === importer else { return }
            self.lastOpenError = error
        }
        if let present = imageImportPresenter { present(importer) }
        else if let window { importer.present(from: window) }
        return true
    }
    private func startPDFImageExport() -> Bool {
        guard pdfImageExportController?.isRunning != true,
              let workbench = activeWorkbench,
              let document = workbench.workspace.document,
              workbench.workspace.allowsCopying,
              !workbench.workspace.isEncryptedSource,
              let data = document.dataRepresentation(),
              data.count <= 256 * 1024 * 1024 else { return false }
        let window = view.window
        guard pdfImageExportPresenter != nil || (window != nil && window?.attachedSheet == nil) else { return false }
        let title = documentTabs.first(where: { $0.id == activeTabID })?.title ?? "PDF"
        let base = URL(fileURLWithPath: title).deletingPathExtension().lastPathComponent
        let exporter = PDFImageExportController(documentData: data, pageCount: document.pageCount, suggestedBaseName: base)
        pdfImageExportController = exporter
        exporter.onFinished = { [weak self, weak exporter] result in
            guard let self, let exporter, self.pdfImageExportController === exporter else { return }
            self.activeWorkbench?.setInteractionStatus("已导出 \(result.files.count) 张图片 · \(result.directory.path)")
        }
        exporter.onFailure = { [weak self, weak exporter] error in
            guard let self, let exporter, self.pdfImageExportController === exporter else { return }
            self.lastOpenError = error
            self.activeWorkbench?.setInteractionStatus(error.localizedDescription)
        }
        exporter.onDismiss = { [weak self, weak exporter] in
            guard let self, let exporter, self.pdfImageExportController === exporter else { return }
            self.pdfImageExportController = nil
        }
        if let present = pdfImageExportPresenter { present(exporter) }
        else if let window { exporter.present(from: window) }
        return true
    }
    func activate(tabID: UUID) {
        if activeTabID != tabID, activeWorkbench?.pdfView.finishInlineTextEditing() == false { return }
        if activeTabID != tabID, let active = activeWorkbench,
           active.isLocalToolRunning || active.isContentEditPending { return }
        // Loading the shell establishes its home state. Do that before assigning
        // the active tab, not later while transferring keyboard focus.
        _ = view
        if activeTabID != tabID { invalidateAIContext() }
        if let canvas = canvasTabs.first(where: { $0.id == tabID }) {
            activeWorkbench?.registerInkMenuItem(nil)
            activeTabID = tabID; lastActiveTabID = tabID
            rebuildTabs(); show(canvas.controller); onActiveWorkbenchChange?(nil)
            view.window?.makeFirstResponder(canvas.controller.canvasView)
            return
        }
        guard documentTabs.contains(where: { $0.id == tabID }) else { return }
        activeWorkbench?.registerInkMenuItem(nil)
        activeTabID = tabID; lastActiveTabID = tabID
        activeWorkbench?.onShowCopilot = { [weak self] in _ = self?.showCopilot() }
        activeWorkbench?.onDocumentSave = { [weak self] in
            guard let self else { return }
            if let url = self.activeWorkbench?.workspace.sourceURL { try? self.recentStore.recordSuccessfulOpen(url) }
            self.rebuildTabs()
        }
        activeWorkbench?.onDocumentRevisionChange = { [weak self] in self?.copilotController?.invalidateContext() }
        activeWorkbench?.readerCommands.externalCommand = { [weak self] command in self?.runReaderTabCommand(command) ?? false }
        activeWorkbench?.readerCommands.canExternalCommand = { [weak self] command in
            guard let self, self.activeTabID == tabID else { return false }
            return command == .newTab || self.documentTabs.count > 1
        }
        rebuildTabs(); show(activeWorkbench!); onActiveWorkbenchChange?(activeWorkbench)
    }
    func showHome() {
        guard activeWorkbench?.pdfView.finishInlineTextEditing() != false else { return }
        guard activeWorkbench?.view.window?.attachedSheet == nil else { return }
        if activeTabID != nil { invalidateAIContext() }
        activeWorkbench?.registerInkMenuItem(nil)
        activeTabID = nil; rebuildTabs(); show(dashboard)
        dashboard.refreshContents(); onActiveWorkbenchChange?(nil)
    }
    @discardableResult func routeDocumentCommand(_ action: Selector, sender: NSMenuItem? = nil) -> Bool {
        guard let workbench = activeWorkbench, workbench.responds(to: action) else { return false }
        let item = NSMenuItem(title: "", action: action, keyEquivalent: "")
        guard workbench.validateMenuItem(item) else { return false }
        if NSStringFromSelector(action).contains(":") { workbench.perform(action, with: sender ?? item) }
        else { workbench.perform(action) }
        return true
    }
    @discardableResult func close(tabID: UUID, dirtyDecision: (() -> Bool)? = nil) -> Bool {
        if let index = canvasTabs.firstIndex(where: { $0.id == tabID }) {
            let canvas = canvasTabs[index].controller
            canvas.finishEditing()
            guard !canvas.session.isDirty || (dirtyDecision?() ?? canvas.guardDirty()) else { return false }
            canvasTabs.remove(at: index)
            if activeTabID == tabID {
                if let next = canvasTabs.last { activate(tabID: next.id) }
                else if let next = documentTabs.last { activate(tabID: next.id) }
                else { showHome() }
            } else { rebuildTabs() }
            return true
        }
        guard let index = documentTabs.firstIndex(where: { $0.id == tabID }) else { return true }
        let controller = documentTabs[index].controller
        guard controller.pdfView.finishInlineTextEditing() else { return false }
        guard !controller.isLocalToolRunning, controller.view.window?.attachedSheet == nil else { return false }
        guard !controller.workspace.isDirty || (dirtyDecision?() ?? controller.guardDirty()) else { return false }
        controller.readerSession.cancelSearch()
        controller.cancelLocalTool()
        documentTabs.remove(at: index)
        if activeTabID == tabID { if let next = documentTabs[safe: min(index, max(documentTabs.count - 1, 0))] { activate(tabID: next.id) } else if let next = canvasTabs.last { activate(tabID: next.id) } else { showHome() } }
        else { rebuildTabs() }
        return true
    }
    func guardAllDirty(_ decision: ((DocumentTab) -> Bool)? = nil) -> Bool {
        guardAllDirty(canvasDecision: nil, decision)
    }
    func guardAllDirty(canvasDecision: ((CanvasTab) -> Bool)?, _ decision: ((DocumentTab) -> Bool)? = nil) -> Bool {
        canvasTabs.forEach { $0.controller.finishEditing() }
        guard documentTabs.allSatisfy({ $0.controller.pdfView.finishInlineTextEditing() }) else { return false }
        guard documentTabs.allSatisfy({ !$0.controller.isLocalToolRunning && $0.controller.view.window?.attachedSheet == nil }) else { return false }
        return documentTabs.allSatisfy { !$0.controller.workspace.isDirty || (decision?($0) ?? $0.controller.guardDirty()) }
        && canvasTabs.allSatisfy { !$0.controller.session.isDirty || (canvasDecision?($0) ?? $0.controller.guardDirty()) }
    }

    @discardableResult private func runReaderTabCommand(_ command: ReaderCommand) -> Bool {
        switch command {
        case .newTab: showHome(); return true
        case .nextTab, .previousTab:
            guard let next = ReaderTabPolicy.adjacent(ids: documentTabs.map(\.id), active: activeTabID, backward: command == .previousTab) else { return false }
            activate(tabID: next); return true
        case .closeOtherTabs:
            guard let activeTabID, let closing = ReaderTabPolicy.closeOthers(ids: documentTabs.map(\.id), keeping: activeTabID, approve: { id in
                guard let tab = self.documentTabs.first(where: { $0.id == id }) else { return false }
                guard tab.controller.pdfView.finishInlineTextEditing() else { return false }
                return !tab.controller.workspace.isDirty || tab.controller.guardDirty()
            }) else { return false }
            for tab in documentTabs where closing.contains(tab.id) { tab.controller.readerSession.cancelSearch(); tab.controller.cancelLocalTool() }
            documentTabs.removeAll { closing.contains($0.id) }
            activate(tabID: activeTabID); return true
        default: return false
        }
    }

    @discardableResult func dispatch(toolID: String, openProvider: () -> URL?) -> Bool {
        guard let descriptor = ToolCatalog.descriptor(id: toolID), descriptor.availability != .unavailable else { return false }
        if toolID == "blank" { _ = createBlankDocument(); return true }
        if toolID == "image-to-pdf" { return startImageImport() }
        if activeTabID == nil, let lastActiveTabID, documentTabs.contains(where: { $0.id == lastActiveTabID }) { activate(tabID: lastActiveTabID) }
        if activeWorkbench == nil {
            guard let url = openProvider(), open(url: url) else { return false }
        }
        guard let workbench = activeWorkbench else { return false }
        if toolID == "copilot" { return showCopilot() }
        if toolID == "pdf-to-image" { return startPDFImageExport() }
        switch toolID {
        case "annotate", "comments": workbench.enterMode(.annotation)
        case "edit": workbench.enterMode(.editing)
        case "redact": workbench.enterMode(.editing); workbench.selectContentTool("content-redact")
        case "sign": workbench.enterMode(.signature)
        case "pages", "merge": workbench.enterMode(.pages)
        case "password": workbench.enterMode(.export)
        case "draw": workbench.enterMode(.drawing)
        case "page-numbers", "compress", "ocr": workbench.presentLocalTool(toolID)
        default: return false
        }
        return true
    }

    private func show(_ controller: NSViewController) {
        children.forEach { $0.removeFromParent() }; content.subviews.forEach { $0.removeFromSuperview() }
        addChild(controller); let child = controller.view; child.translatesAutoresizingMaskIntoConstraints = false; content.addSubview(child)
        NSLayoutConstraint.activate([child.leadingAnchor.constraint(equalTo: content.leadingAnchor), child.trailingAnchor.constraint(equalTo: content.trailingAnchor), child.topAnchor.constraint(equalTo: content.topAnchor), child.bottomAnchor.constraint(equalTo: content.bottomAnchor)])
    }
    private func rebuildTabs() {
        tabBar.arrangedSubviews.forEach { tabBar.removeArrangedSubview($0); $0.removeFromSuperview() }
        let home = ChromeTabButton(title: "", target: self, action: #selector(homeAction)); home.image = HomeBrandIcon.image(); home.isBordered = false; home.isSelectedTab = activeTabID == nil; home.setAccessibilityLabel("主界面标签"); home.toolTip = "主界面"; home.widthAnchor.constraint(equalToConstant: 44).isActive = true; home.heightAnchor.constraint(equalToConstant: 40).isActive = true; homeTab = home; tabBar.addArrangedSubview(home)
        for tab in documentTabs {
            let box = NSStackView(); box.orientation = .horizontal; box.spacing = 4
            box.wantsLayer = true; box.layer?.backgroundColor = (activeTabID == tab.id ? NSColor(calibratedWhite: 0.25, alpha: 1) : ShellPalette.top).cgColor
            let select = TabButton(title: tab.title, target: self, action: #selector(selectTab(_:))); select.tabID = tab.id; select.isBordered = false; select.state = activeTabID == tab.id ? .on : .off; select.font = .systemFont(ofSize: 12); select.contentTintColor = ShellPalette.primary; select.setAccessibilityLabel("文档标签 \(tab.title)"); select.setAccessibilityValue(activeTabID == tab.id ? "已选中" : "未选中")
            let close = TabButton(title: "×", target: self, action: #selector(closeTab(_:))); close.tabID = tab.id; close.isBordered = false; close.setAccessibilityLabel("关闭 \(tab.title)")
            configureTabLayout(box, select: select, close: close)
            box.heightAnchor.constraint(equalToConstant: 40).isActive = true
            box.addArrangedSubview(select); box.addArrangedSubview(close); tabBar.addArrangedSubview(box)
        }
        for tab in canvasTabs {
            let box = NSStackView(); box.orientation = .horizontal; box.spacing = 4
            box.wantsLayer = true; box.layer?.backgroundColor = (activeTabID == tab.id ? NSColor(calibratedWhite: 0.25, alpha: 1) : ShellPalette.top).cgColor
            let select = TabButton(title: "无边记 · \(tab.title)", target: self, action: #selector(selectTab(_:)))
            select.tabID = tab.id; select.isBordered = false; select.state = activeTabID == tab.id ? .on : .off
            select.font = .systemFont(ofSize: 12); select.contentTintColor = ShellPalette.primary
            select.setAccessibilityLabel("无边记标签 \(tab.title)")
            let close = TabButton(title: "×", target: self, action: #selector(closeTab(_:)))
            close.tabID = tab.id; close.isBordered = false; close.setAccessibilityLabel("关闭无边记 \(tab.title)")
            configureTabLayout(box, select: select, close: close)
            box.heightAnchor.constraint(equalToConstant: 40).isActive = true
            box.addArrangedSubview(select); box.addArrangedSubview(close); tabBar.addArrangedSubview(box)
        }
        let plus = ChromeTabButton(title: "+", target: self, action: #selector(homeAction)); plus.isBordered = false; plus.font = .systemFont(ofSize: 24, weight: .light); plus.setAccessibilityLabel("新标签"); plus.toolTip = "新标签"; plus.widthAnchor.constraint(equalToConstant: 36).isActive = true; plus.heightAnchor.constraint(equalToConstant: 40).isActive = true; newTabButton = plus; tabBar.addArrangedSubview(plus)
        // Only this trailing space may grow. Otherwise AppKit stretches the first
        // document tab to fill the titlebar and pushes its neighbour to the right.
        let trailingSpace = NSView()
        trailingSpace.setContentHuggingPriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        trailingSpace.setContentCompressionResistancePriority(NSLayoutConstraint.Priority(1), for: .horizontal)
        tabBar.addArrangedSubview(trailingSpace)
    }
    private func configureTabLayout(_ box: NSStackView, select: NSButton, close: NSButton) {
        select.lineBreakMode = .byTruncatingMiddle
        select.toolTip = select.title
        select.setContentHuggingPriority(.required, for: .horizontal)
        box.setContentHuggingPriority(.required, for: .horizontal)
        box.widthAnchor.constraint(lessThanOrEqualToConstant: 360).isActive = true
        close.widthAnchor.constraint(equalToConstant: 20).isActive = true
    }
    @objc private func homeAction() { dashboard.select(.home); showHome() }
    @objc private func selectTab(_ sender: TabButton) { if let id = sender.tabID { activate(tabID: id) } }
    @objc private func closeTab(_ sender: TabButton) { if let id = sender.tabID { _ = close(tabID: id) } }
}

enum HomeBrandIcon {
    static func image() -> NSImage? {
        if let url = Bundle.main.resourceURL?.appendingPathComponent("PDFMaster.icns"), let image = NSImage(contentsOf: url) { return image }
        // Unit tests and swift run have no app resource bundle.
        let source = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("Resources/PDFMaster.icns")
        return NSImage(contentsOf: source) ?? NSImage(systemSymbolName: "doc.fill", accessibilityDescription: "PDF Master")
    }
}
private final class ChromeTabButton: NSButton {
    var isSelectedTab = false
    override func draw(_ dirtyRect: NSRect) {
        (isSelectedTab || isHighlighted ? NSColor(calibratedWhite: 0.25, alpha: 1) : ShellPalette.top).setFill(); bounds.fill()
        if let image { image.draw(in: NSRect(x: (bounds.width - 20) / 2, y: (bounds.height - 20) / 2, width: 20, height: 20)) }
        else {
            let attributes: [NSAttributedString.Key: Any] = [.font: font ?? NSFont.systemFont(ofSize: 24), .foregroundColor: ShellPalette.secondary]
            let size = (title as NSString).size(withAttributes: attributes)
            (title as NSString).draw(at: NSPoint(x: (bounds.width-size.width)/2, y: (bounds.height-size.height)/2), withAttributes: attributes)
        }
    }
}

private final class TabButton: NSButton { var tabID: UUID? }
private extension Array { subscript(safe index: Int) -> Element? { indices.contains(index) ? self[index] : nil } }
