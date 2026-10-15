import AppKit
import PaperCore
import UniformTypeIdentifiers

private final class CanvasDashboardButton: NSButton { var tabID: UUID? }

enum HomeSection: String, CaseIterable { case home = "主界面", tools = "工具", canvas = "无限画布", recent = "最近", favorites = "收藏", account = "账号详情", settings = "设置", support = "获取支持" }

@MainActor
final class DashboardViewController: NSViewController {
    unowned let shell: ApplicationShellController
    private let detail = NSStackView()
    private lazy var iconSkins = ToolIconSkinStore(defaults: .standard, resourceRoot: Bundle.main.resourceURL, toolIDs: ToolCatalog.all.map(\.id))
    private var columnWidth: NSLayoutConstraint?
    private var columnTop: NSLayoutConstraint?
    private var lastLayoutWidth: CGFloat = -1
    private weak var sectionScrollView: NSScrollView?
    private var canvasWebsite: CanvasWebsiteController?
    private var lastRenderedSection: HomeSection?
    private var toolCards: [NSView] = []
    private weak var homeDropZone: HomeDropZoneView?
    private weak var homeOpenButton: DashboardActionButton?
    private weak var homeAllToolsButton: DashboardActionButton?
    private weak var homeCustomizeButton: DashboardActionButton?
    private weak var fileImportStatusRow: NSStackView?
    private weak var fileImportProgress: NSProgressIndicator?
    private weak var fileImportStatusLabel: NSTextField?
    private weak var fileImportCancelButton: DashboardActionButton?
    private var emptyIllustrations: [HomeFolderIllustration] = []
    var contentColumnFrame: NSRect { detail.convert(detail.bounds, to: view) }
    var toolCardFrames: [NSRect] { toolCards.map { $0.convert($0.bounds, to: view) } }
    var toolCardIconCount: Int { toolCards.compactMap { $0 as? ToolButton }.filter { $0.iconView.image != nil && $0.iconView.frame.height > 20 }.count }
    var homeDropZoneFrame: NSRect { homeDropZone.map { $0.convert($0.bounds, to: view) } ?? .zero }
    var homeDropZoneHasDashedBorder: Bool { homeDropZone?.usesDashedBorder == true }
    var homeDropZoneHasPDFGlyph: Bool { homeDropZone?.subviews.contains(where: { $0 is HomePDFStackGlyph }) == true }
    var homeOpenButtonStyle: String? { homeOpenButton?.style.rawValue }
    var homeOpenButtonHasAction: Bool { homeOpenButton?.target === self && homeOpenButton?.action == #selector(openFile) }
    var homeAllToolsButtonStyle: String? { homeAllToolsButton?.style.rawValue }
    var homeAllToolsButtonIsBorderless: Bool { homeAllToolsButton?.isBordered == false }
    var homeCustomizeButtonStyle: String? { homeCustomizeButton?.style.rawValue }
    var homeNavigationUsesBrandIcon: Bool { navigationButtons[.home]?.image?.name() == "PDFMasterBrand" }
    var homeHasSubscriptionTrialButton: Bool { detail.arrangedSubviews.contains { ($0 as? NSButton)?.title.contains("试用") == true } }
    var homeEmptyIllustrationKinds: [String] { emptyIllustrations.map(\.kind) }
    var homeEmptyIllustrationFrames: [NSRect] { emptyIllustrations.map { $0.convert($0.bounds, to: view) } }
    func activateAllToolsForTesting() { homeAllToolsButton?.performClick(nil) }
    private var navigationButtons: [HomeSection: NSButton] = [:]
    private var section: HomeSection = .home
    private(set) var renderedColumnCount = 6
    var selectedSection: HomeSection { section }
    let acceptsLocalPDFDrop = true
    init(shell: ApplicationShellController) {
        self.shell = shell
        super.init(nibName: nil, bundle: nil)
        shell.onFileImportStateChange = { [weak self] _ in self?.updateFileImportStatus() }
        NotificationCenter.default.addObserver(self, selector: #selector(iconSkinChanged), name: ToolIconSkinStore.didChange, object: nil)
    }
    deinit { NotificationCenter.default.removeObserver(self) }
    @objc private func iconSkinChanged() { if isViewLoaded { render() } }
    required init?(coder: NSCoder) { fatalError() }
    override func loadView() {
        let root = PDFDropView(); root.wantsLayer = true; root.layer?.backgroundColor = ShellPalette.canvas.cgColor
        root.onFiles = { [weak self] urls in
            guard let self else { return false }
            guard !self.shell.isFileImportRunning else { return false }
            if urls.count == 1, urls[0].pathExtension.lowercased() == "pdf" { return self.openLocalPDF(urls[0]) }
            Task {
                do { _ = try await self.shell.importDroppedFiles(urls) }
                catch FilePDFImportError.cancelled { }
                catch { self.showPersistenceError("无法导入文件", error) }
            }
            return true
        }
        root.onResize = { [weak self] in self?.updateColumnsIfNeeded() }
        let sidebar = NSStackView(); sidebar.orientation = .vertical; sidebar.spacing = 0; sidebar.edgeInsets = NSEdgeInsets(top: 14, left: 14, bottom: 18, right: 14); sidebar.wantsLayer = true; sidebar.layer?.backgroundColor = ShellPalette.sidebar.cgColor
        for item in [HomeSection.home, .tools, .canvas, .recent, .favorites] { sidebar.addArrangedSubview(navButton(item)) }
        sidebar.addArrangedSubview(NSView()); for item in [HomeSection.account, .settings, .support] { sidebar.addArrangedSubview(navButton(item)) }
        detail.orientation = .vertical; detail.alignment = .leading; detail.spacing = 20
        let host = FlippedDashboardView(); host.translatesAutoresizingMaskIntoConstraints = false; detail.translatesAutoresizingMaskIntoConstraints = false; host.addSubview(detail)
        let width = detail.widthAnchor.constraint(equalToConstant: 880); columnWidth = width
        let top = detail.topAnchor.constraint(equalTo: host.topAnchor, constant: 42); columnTop = top
        let scroll = NSScrollView(); scroll.hasVerticalScroller = true; scroll.drawsBackground = false; scroll.documentView = host
        sectionScrollView = scroll
        [sidebar, scroll].forEach { $0.translatesAutoresizingMaskIntoConstraints = false; root.addSubview($0) }
        NSLayoutConstraint.activate([
            sidebar.leadingAnchor.constraint(equalTo: root.leadingAnchor), sidebar.topAnchor.constraint(equalTo: root.topAnchor), sidebar.bottomAnchor.constraint(equalTo: root.bottomAnchor), sidebar.widthAnchor.constraint(equalToConstant: 216),
            scroll.leadingAnchor.constraint(equalTo: sidebar.trailingAnchor), scroll.trailingAnchor.constraint(equalTo: root.trailingAnchor), scroll.topAnchor.constraint(equalTo: root.topAnchor), scroll.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            host.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor), host.heightAnchor.constraint(greaterThanOrEqualTo: scroll.contentView.heightAnchor),
            top, detail.bottomAnchor.constraint(equalTo: host.bottomAnchor, constant: -36), detail.centerXAnchor.constraint(equalTo: host.centerXAnchor), width
        ])
        view = root; render()
    }
    private func navButton(_ item: HomeSection) -> NSButton {
        let button = NavButton(title: item.rawValue, target: self, action: #selector(selectSection(_:)))
        let symbols: [HomeSection: String] = [.tools: "square.3.layers.3d", .canvas: "infinity", .recent: "clock.fill", .favorites: "star.fill", .account: "person", .settings: "gearshape", .support: "questionmark.circle"]
        button.image = item == .home ? Self.brandIcon() : NSImage(systemSymbolName: symbols[item]!, accessibilityDescription: item.rawValue)
        button.imagePosition = .imageLeading; button.section = item; button.bezelStyle = .recessed
        button.toolTip = item.rawValue
        button.alignment = .left; button.font = .systemFont(ofSize: 14)
        button.heightAnchor.constraint(equalToConstant: 40).isActive = true
        button.widthAnchor.constraint(equalToConstant: 188).isActive = true
        navigationButtons[item] = button
        return button
    }
    @objc private func selectSection(_ sender: NavButton) { section = sender.section ?? .home; render() }
    func select(_ value: HomeSection) { section = value; render() }
    func refreshResponsiveLayout() { render() }
    private func updateColumnsIfNeeded() {
        guard view.bounds.width != lastLayoutWidth else { return }
        lastLayoutWidth = view.bounds.width
        columnWidth?.constant = max(136, min(880, view.bounds.width - 216 - 64))
        render()
    }
    func refreshContents() { render() }
    private func render() {
        columnWidth?.constant = max(136, min(880, view.bounds.width - 216 - 64))
        detail.spacing = section == .home ? 24 : 20; columnTop?.constant = section == .home ? 42 : 36
        toolCards.removeAll()
        emptyIllustrations.removeAll()
        for (item, button) in navigationButtons { button.state = item == section ? .on : .off; button.contentTintColor = item == section ? ShellPalette.accent : ShellPalette.primary }
        detail.arrangedSubviews.forEach { detail.removeArrangedSubview($0); $0.removeFromSuperview() }
        sectionScrollView?.isHidden = section == .canvas
        canvasWebsite?.view.isHidden = section != .canvas
        if section == .canvas {
            if canvasWebsite == nil, let scroll = sectionScrollView {
                let website = CanvasWebsiteController(); canvasWebsite = website; addChild(website)
                let hosted = website.view; hosted.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(hosted)
                NSLayoutConstraint.activate([
                    hosted.leadingAnchor.constraint(equalTo: scroll.leadingAnchor), hosted.trailingAnchor.constraint(equalTo: scroll.trailingAnchor),
                    hosted.topAnchor.constraint(equalTo: scroll.topAnchor), hosted.bottomAnchor.constraint(equalTo: scroll.bottomAnchor)
                ])
            }
            lastRenderedSection = section
            sectionScrollView?.contentView.scroll(to: .zero)
            return
        }
        switch section {
        case .home: renderHome()
        case .tools: toolsHeader(); ToolGroup.allCases.forEach { group in heading(group.rawValue); toolGrid(ToolCatalog.all.filter { $0.group == group }) }
        case .canvas: break // The website fills the workspace, outside the dashboard's scroll column.
        case .recent: title("最近"); if let error = shell.recentStore.lastError { paragraph("最近记录读取失败：\(error.localizedDescription)") }; documentList(shell.recentStore.recents, empty: "尚无最近打开的文档。打开 PDF 后会在这里显示。", clear: true)
        case .favorites: title("收藏"); documentList(shell.recentStore.favorites, empty: "尚无收藏。可在最近文档中标记常用文件。", clear: false)
        case .account:
            title("账号详情")
            paragraph(shell.accountStore.currentUsername.map { "当前本机账户：\($0)" } ?? "尚未登录本机账户。阅读与批注无需登录；AI 使用当前账户的 Key。")
            paragraph("账户仅用于此 Mac，不是云端账户。打开文档不会自动上传；AI 需要另行确认发送范围与可能的计费。")
            detail.addArrangedSubview(NSButton(title: "管理本机账户…", target: self, action: #selector(openAccountSettings)))
        case .settings: renderSettings()
        case .support: title("获取支持"); paragraph("需要帮助？请提供操作步骤、文件类型和错误提示。请勿发送密码、API Key 或敏感文档；应用不会自动上传文件。")
        }
        for child in detail.arrangedSubviews where !(child is NSGridView) {
            child.widthAnchor.constraint(equalTo: detail.widthAnchor).isActive = true
        }
        if lastRenderedSection != section {
            lastRenderedSection = section
            if let scroll = sectionScrollView {
                scroll.contentView.scroll(to: .zero)
                scroll.reflectScrolledClipView(scroll.contentView)
            }
        }
    }
    private func toolsHeader() {
        let row = NSStackView(); row.orientation = .horizontal; row.alignment = .centerY; row.spacing = 12
        row.addArrangedSubview(label("推荐上手工具", 28, .semibold)); row.addArrangedSubview(NSView())
        row.addArrangedSubview(ToolIconSkinButton(store: iconSkins)); detail.addArrangedSubview(row)
    }
    private func renderSettings() {
        title("设置")
        heading("本机账户与 AI")
        paragraph("DMX Key 按账户保存在 macOS 钥匙串，不写入最近记录或普通偏好设置。")
        detail.addArrangedSubview(NSButton(title: "输入 / 管理 DMX Key…", target: self, action: #selector(openAccountSettings)))
        paragraph("推荐工具会保存在本机 JSON 设置中。文档内容、PDF 密码和凭据不会写入最近记录。")
        heading("工具图标")
        let row = NSStackView(); row.orientation = .horizontal; row.alignment = .centerY; row.spacing = 12
        let text = label("当前：\(iconSkins.selected.title) · 皮肤只影响工具图标", 13, .regular, ShellPalette.secondary)
        row.addArrangedSubview(text); row.addArrangedSubview(NSView()); row.addArrangedSubview(ToolIconSkinButton(store: iconSkins))
        detail.addArrangedSubview(row)
        if let error = shell.preferredTools.lastError { paragraph("推荐工具设置读取失败：\(error.localizedDescription)") }
    }
    @objc private func openAccountSettings() { shell.showAccountSettings() }
    private func renderHome() {
        let row = NSStackView(); row.orientation = .horizontal; row.alignment = .centerY; row.addArrangedSubview(label("主界面", 32, .semibold)); row.addArrangedSubview(NSView())
        let customize = DashboardActionButton(title: "自定义", style: .neutralPill, target: self, action: #selector(customizeTools)); customize.widthAnchor.constraint(equalToConstant: 60).isActive = true; customize.heightAnchor.constraint(equalToConstant: 32).isActive = true; homeCustomizeButton = customize; row.addArrangedSubview(customize); detail.addArrangedSubview(row)
        let banner = card(); let bannerText = NSTextField(wrappingLabelWithString: "PDF Master · 阅读、批注、整理，让文档与灵感井然有序"); bannerText.textColor = ShellPalette.primary; banner.addSubviewPinned(bannerText, inset: 18); banner.heightAnchor.constraint(equalToConstant: 72).isActive = true; detail.addArrangedSubview(banner)
        let drop = HomeDropZoneView(); homeDropZone = drop
        let glyph = HomePDFStackGlyph(); let open = DashboardActionButton(title: "打开文件", style: .bluePill, target: self, action: #selector(openFile)); homeOpenButton = open
        let caption = label("拖入 PDF、图片或文字文件，可混合合并", 12, .regular, NSColor(calibratedWhite: 0.4, alpha: 1))
        for child in [glyph, open, caption] { child.translatesAutoresizingMaskIntoConstraints = false; drop.addSubview(child) }
        NSLayoutConstraint.activate([
            drop.heightAnchor.constraint(equalToConstant: 190),
            glyph.centerXAnchor.constraint(equalTo: drop.centerXAnchor), glyph.topAnchor.constraint(equalTo: drop.topAnchor, constant: 27), glyph.widthAnchor.constraint(equalToConstant: 85), glyph.heightAnchor.constraint(equalToConstant: 64),
            open.centerXAnchor.constraint(equalTo: drop.centerXAnchor), open.topAnchor.constraint(equalTo: glyph.bottomAnchor, constant: 18), open.widthAnchor.constraint(equalToConstant: 88), open.heightAnchor.constraint(equalToConstant: 30),
            caption.centerXAnchor.constraint(equalTo: drop.centerXAnchor), caption.topAnchor.constraint(equalTo: open.bottomAnchor, constant: 6)
        ]);
        let progressRow = NSStackView(); progressRow.orientation = .horizontal; progressRow.alignment = .centerY; progressRow.spacing = 8
        progressRow.identifier = NSUserInterfaceItemIdentifier("home-file-import-status")
        let spinner = NSProgressIndicator(); spinner.style = .spinning; spinner.controlSize = .small; spinner.isDisplayedWhenStopped = false
        let progressLabel = label("正在本地导入文件…", 11, .regular, ShellPalette.secondary)
        progressLabel.lineBreakMode = .byTruncatingTail; progressLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let cancel = DashboardActionButton(title: "取消", style: .neutralPill, target: self, action: #selector(cancelFileImport))
        cancel.identifier = NSUserInterfaceItemIdentifier("home-file-import-cancel")
        cancel.toolTip = "取消当前文件导入，不修改原文件"
        for child in [spinner, progressLabel, cancel] { progressRow.addArrangedSubview(child) }
        progressRow.translatesAutoresizingMaskIntoConstraints = false; drop.addSubview(progressRow)
        NSLayoutConstraint.activate([
            progressRow.centerXAnchor.constraint(equalTo: drop.centerXAnchor), progressRow.bottomAnchor.constraint(equalTo: drop.bottomAnchor, constant: -4),
            progressRow.leadingAnchor.constraint(greaterThanOrEqualTo: drop.leadingAnchor, constant: 12), progressRow.trailingAnchor.constraint(lessThanOrEqualTo: drop.trailingAnchor, constant: -12),
            spinner.widthAnchor.constraint(equalToConstant: 12), spinner.heightAnchor.constraint(equalToConstant: 12),
            cancel.widthAnchor.constraint(equalToConstant: 46), cancel.heightAnchor.constraint(equalToConstant: 24)
        ])
        fileImportStatusRow = progressRow; fileImportProgress = spinner; fileImportStatusLabel = progressLabel; fileImportCancelButton = cancel
        updateFileImportStatus()
        detail.addArrangedSubview(drop)
        let toolsRow = NSStackView(); toolsRow.orientation = .horizontal; toolsRow.alignment = .centerY; toolsRow.addArrangedSubview(label("工具专为您", 20, .semibold)); toolsRow.addArrangedSubview(NSView())
        let all = DashboardActionButton(title: "查看全部", style: .blueLink, target: self, action: #selector(showTools)); homeAllToolsButton = all; all.widthAnchor.constraint(equalToConstant: 76).isActive = true; all.heightAnchor.constraint(equalToConstant: 28).isActive = true; toolsRow.addArrangedSubview(all); detail.addArrangedSubview(toolsRow)
        toolGrid(shell.preferredTools.toolIDs.compactMap(ToolCatalog.descriptor))
        heading("最近"); if shell.recentStore.recents.isEmpty { emptyCollection(kind: "recent-folder", caption: "全部打开的文件将显示在此处") } else { documentList(Array(shell.recentStore.recents.prefix(5)), empty: "", clear: false) }
        heading("收藏"); if shell.recentStore.favorites.isEmpty { emptyCollection(kind: "favorite-folder", caption: "点击任何最近文件的星标，即可将其添加到“收藏”中") } else { documentList(Array(shell.recentStore.favorites.prefix(5)), empty: "", clear: false) }
    }
    private static func brandIcon() -> NSImage? {
        if let cached = NSImage(named: "PDFMasterBrand") { return cached }
        let developmentURL = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("Resources/PDFMaster.icns")
        let urls = [Bundle.main.resourceURL?.appendingPathComponent("PDFMaster.icns"), developmentURL].compactMap { $0 }
        guard let image = urls.lazy.compactMap({ NSImage(contentsOf: $0) }).first else { return nil }
        image.isTemplate = false; image.setName("PDFMasterBrand"); return image
    }
    private func emptyCollection(kind: String, caption: String) {
        let host = NSView(); host.heightAnchor.constraint(equalToConstant: 216).isActive = true
        let folder = HomeFolderIllustration(kind: kind); emptyIllustrations.append(folder)
        let text = NSTextField(wrappingLabelWithString: caption); text.font = .systemFont(ofSize: 13); text.textColor = ShellPalette.secondary; text.alignment = .center
        for child in [folder, text] { child.translatesAutoresizingMaskIntoConstraints = false; host.addSubview(child) }
        NSLayoutConstraint.activate([
            folder.centerXAnchor.constraint(equalTo: host.centerXAnchor), folder.centerYAnchor.constraint(equalTo: host.centerYAnchor, constant: -18), folder.widthAnchor.constraint(equalToConstant: 78), folder.heightAnchor.constraint(equalToConstant: 54),
            text.centerXAnchor.constraint(equalTo: host.centerXAnchor), text.topAnchor.constraint(equalTo: folder.bottomAnchor, constant: 16), text.leadingAnchor.constraint(greaterThanOrEqualTo: host.leadingAnchor, constant: 8), text.trailingAnchor.constraint(lessThanOrEqualTo: host.trailingAnchor, constant: -8)
        ]); detail.addArrangedSubview(host)
    }
    private func toolGrid(_ tools: [ToolDescriptor]) {
        let available = columnWidth?.constant ?? 880
        let cardWidth = section == .home ? 140 : 136
        let columns = max(1, min(6, Int((available + 8) / CGFloat(cardWidth + 8))))
        renderedColumnCount = columns
        let grid = NSGridView(); grid.xPlacement = .leading; grid.columnSpacing = 8; grid.rowSpacing = 8
        for start in stride(from: 0, to: tools.count, by: columns) { grid.addRow(with: Array(tools[start..<min(start + columns, tools.count)]).map(toolButton)) }
        for index in 0..<grid.numberOfColumns { grid.column(at: index).width = CGFloat(cardWidth) }
        grid.widthAnchor.constraint(equalToConstant: CGFloat(grid.numberOfColumns * cardWidth + max(0, grid.numberOfColumns - 1) * 8)).isActive = true
        detail.addArrangedSubview(grid)
    }
    private func toolButton(_ tool: ToolDescriptor) -> NSView {
        let button = ToolButton(title: tool.title, target: self, action: #selector(runTool(_:)))
        button.toolID = tool.id
        let generated = iconSkins.image(id: tool.id)
        button.image = generated ?? NSImage(systemSymbolName: tool.symbol, accessibilityDescription: tool.title)?.withSymbolConfiguration(.init(pointSize: 28, weight: .medium))
        button.imagePosition = .imageAbove
        button.contentTintColor = tool.tint
        button.font = .systemFont(ofSize: 14)
        button.bezelStyle = .recessed
        button.wantsLayer = true
        button.layer?.backgroundColor = ShellPalette.card.cgColor
        button.layer?.cornerRadius = 18
        button.toolTip = tool.unavailableReason ?? tool.title
        button.setAccessibilityHelp(button.toolTip)
        button.setAccessibilityLabel(tool.title)
        button.widthAnchor.constraint(equalToConstant: section == .home ? 140 : 136).isActive = true
        button.heightAnchor.constraint(equalToConstant: section == .home ? 120 : 116).isActive = true
        button.isEnabled = tool.availability != .unavailable
        button.configureIconAndTitle(tint: generated == nil ? tool.tint : nil)
        toolCards.append(button)
        return button
    }
    @objc private func runTool(_ sender: ToolButton) {
        guard let id = sender.toolID else { return }
        _ = shell.dispatch(toolID: id) { self.choosePDF() }
    }
    private func updateFileImportStatus() {
        guard isViewLoaded else { return }
        let running = shell.isFileImportRunning
        fileImportStatusRow?.isHidden = !running
        fileImportStatusLabel?.stringValue = "正在本地导入文件…"
        fileImportCancelButton?.isEnabled = running
        if running { fileImportProgress?.startAnimation(nil) }
        else { fileImportProgress?.stopAnimation(nil) }
    }
    @objc private func cancelFileImport() {
        guard shell.isFileImportRunning else { return }
        shell.cancelFileImport()
        fileImportStatusLabel?.stringValue = "正在取消导入…"
        fileImportCancelButton?.isEnabled = false
    }
    @objc private func showTools() { section = .tools; render() }
    @objc private func openFile() { if let url = choosePDF() { _ = openLocalPDF(url) } }
    @objc private func newCanvas() { shell.createCanvas() }
    @objc private func openCanvas() {
        let panel = NSOpenPanel(); panel.canChooseDirectories = false; panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [UTType(filenameExtension: "pdfmasterboard") ?? UTType(exportedAs: "cn.nankong.pdfmaster.board", conformingTo: .data)]
        if panel.runModal() == .OK, let url = panel.url {
            _ = shell.openCanvas(url: url)
            if let error = shell.lastOpenError { showPersistenceError("无法打开画布", error) }
        }
    }
    @objc private func activateCanvas(_ sender: CanvasDashboardButton) { if let id = sender.tabID { shell.activate(tabID: id) } }
    @discardableResult private func openLocalPDF(_ url: URL) -> Bool {
        let result = shell.open(url: url)
        if let error = shell.lastOpenError { showPersistenceError("无法打开文档", error) }
        return result
    }
    private func choosePDF() -> URL? { let panel = NSOpenPanel(); panel.allowedContentTypes = [.pdf]; return panel.runModal() == .OK ? panel.url : nil }
    @objc private func customizeTools() {
        let list = NSStackView(); list.orientation = .vertical; list.spacing = 7
        var popups: [NSPopUpButton] = []
        for (index, selectedID) in shell.preferredTools.toolIDs.enumerated() {
            let row = NSStackView(); row.orientation = .horizontal; row.addArrangedSubview(label("\(index + 1)", 12, .medium, ShellPalette.secondary))
            let popup = NSPopUpButton(); popup.addItems(withTitles: ToolCatalog.all.map(\.title)); popup.selectItem(at: ToolCatalog.all.firstIndex { $0.id == selectedID } ?? 0); popup.widthAnchor.constraint(equalToConstant: 250).isActive = true
            popups.append(popup); row.addArrangedSubview(popup); list.addArrangedSubview(row)
        }
        let alert = NSAlert(); alert.messageText = "自定义推荐工具"; alert.informativeText = "六个位置可独立排序，且不能重复。取消不会更改当前设置。"; alert.accessoryView = list; alert.addButton(withTitle: "保存"); alert.addButton(withTitle: "取消")
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        let selected = popups.map { ToolCatalog.all[$0.indexOfSelectedItem].id }
        do { try shell.preferredTools.replace(with: selected); render() }
        catch { showPersistenceError("无法保存推荐工具，请确认六项不同且设置目录可写", error) }
    }
    private func documentList(_ entries: [RecentDocumentEntry], empty: String, clear: Bool) { if entries.isEmpty { paragraph(empty); return }; for entry in entries { let row = NSStackView(); row.orientation = .horizontal; let title = NSButton(title: entry.normalizedURL.lastPathComponent + (entry.exists ? "" : "（文件缺失）"), target: self, action: #selector(openRecent(_:))); title.identifier = NSUserInterfaceItemIdentifier(entry.normalizedURL.path); row.addArrangedSubview(title); row.addArrangedSubview(NSView()); let favorite = NSButton(title: entry.isFavorite ? "★" : "☆", target: self, action: #selector(toggleFavorite(_:))); favorite.identifier = NSUserInterfaceItemIdentifier(entry.normalizedURL.path); favorite.bezelStyle = .inline; favorite.setAccessibilityLabel(entry.isFavorite ? "取消收藏" : "收藏"); row.addArrangedSubview(favorite); if clear { let remove = NSButton(title: "移除", target: self, action: #selector(removeRecent(_:))); remove.identifier = NSUserInterfaceItemIdentifier(entry.normalizedURL.path); remove.bezelStyle = .inline; row.addArrangedSubview(remove) }; detail.addArrangedSubview(row) }; if clear { let clearButton = NSButton(title: "清除最近记录…", target: self, action: #selector(clearRecents)); detail.addArrangedSubview(clearButton) } }
    @objc private func openRecent(_ sender: NSButton) {
        guard let path = sender.identifier?.rawValue,
              let entry = shell.recentStore.entries.first(where: { $0.normalizedURL.path == path }) else { return }
        do { _ = try shell.recentStore.open(entry) { openLocalPDF($0) } }
        catch { showPersistenceError("无法打开最近文档", error) }
    }
    @objc private func toggleFavorite(_ sender: NSButton) { guard let path = sender.identifier?.rawValue else { return }; do { try shell.recentStore.toggleFavorite(URL(fileURLWithPath: path)); render() } catch { let a = NSAlert(); a.messageText = "无法更新收藏"; a.informativeText = error.localizedDescription; a.runModal() } }
    @objc private func removeRecent(_ sender: NSButton) { guard let path = sender.identifier?.rawValue else { return }; do { try shell.recentStore.removeFromRecents(URL(fileURLWithPath: path)); render() } catch { showPersistenceError("无法移除最近记录", error) } }
    @objc private func clearRecents() { let a = NSAlert(); a.messageText = "清除最近记录？"; a.informativeText = "不会删除原文件；收藏会保留。"; a.addButton(withTitle: "清除"); a.addButton(withTitle: "取消"); guard a.runModal() == .alertFirstButtonReturn else { return }; do { try shell.recentStore.clearRecents(); render() } catch { showPersistenceError("无法清除最近记录", error) } }
    private func showPersistenceError(_ title: String, _ error: Error) { let alert = NSAlert(); alert.messageText = title; alert.informativeText = error.localizedDescription; alert.runModal() }
    private func title(_ text: String) { detail.addArrangedSubview(label(text, 28, .semibold)) }; private func heading(_ text: String) { detail.addArrangedSubview(label(text, 20, .semibold)) }; private func paragraph(_ text: String) { let l = NSTextField(wrappingLabelWithString: text); l.textColor = ShellPalette.secondary; detail.addArrangedSubview(l) }
    private func label(_ text: String, _ size: CGFloat, _ weight: NSFont.Weight, _ color: NSColor = ShellPalette.primary) -> NSTextField { let l = NSTextField(labelWithString: text); l.font = .systemFont(ofSize: size, weight: weight); l.textColor = color; return l }
    private func card() -> NSView { let v = NSView(); v.wantsLayer = true; v.layer?.backgroundColor = ShellPalette.card.cgColor; v.layer?.cornerRadius = 10; return v }
}

private final class NavButton: HoverFeedbackButton {
    var section: HomeSection?
    override var feedbackCornerRadius: CGFloat { 8 }
    override func draw(_ dirtyRect: NSRect) {
        if feedback == .selected || feedback == .pressed || feedback == .hover {
            let fill: NSColor = feedback == .pressed ? ShellPalette.top : feedback == .selected ? NSColor(calibratedWhite: 0.18, alpha: 1) : NSColor(srgbRed: 41.0 / 255, green: 41.0 / 255, blue: 41.0 / 255, alpha: 1)
            fill.setFill()
            NSBezierPath(roundedRect: bounds, xRadius: 8, yRadius: 8).fill()
        }
        let displayedImage = section == .home ? image : image?.withSymbolConfiguration(.init(paletteColors: [state == .on ? ShellPalette.accent : ShellPalette.primary]))
        displayedImage?.draw(in: NSRect(x: 10, y: 11, width: 18, height: 18))
        (title as NSString).draw(in: NSRect(x: 38, y: 10, width: bounds.width - 44, height: 22), withAttributes: [.font: NSFont.systemFont(ofSize: 14), .foregroundColor: ShellPalette.primary])
    }
}
private final class ToolButton: HoverFeedbackButton {
    var toolID: String?
    let iconView = NSImageView()
    private let titleLabel = NSTextField(wrappingLabelWithString: "")
    override var feedbackCornerRadius: CGFloat { 18 }
    override var isEnabled: Bool {
        didSet {
            titleLabel.textColor = isEnabled ? ShellPalette.primary : ShellPalette.secondary
            iconView.alphaValue = isEnabled ? 1 : 0.45
        }
    }
    func configureIconAndTitle(tint: NSColor?) {
        iconView.image = image; iconView.contentTintColor = tint
        iconView.imageScaling = .scaleProportionallyUpOrDown
        titleLabel.stringValue = title; titleLabel.font = .systemFont(ofSize: 14)
        titleLabel.textColor = isEnabled ? ShellPalette.primary : ShellPalette.secondary
        iconView.alphaValue = isEnabled ? 1 : 0.45
        titleLabel.alignment = .center
        for child in [iconView, titleLabel] {
            child.translatesAutoresizingMaskIntoConstraints = false
            child.setAccessibilityElement(false); addSubview(child)
        }
        NSLayoutConstraint.activate([
            iconView.centerXAnchor.constraint(equalTo: centerXAnchor), iconView.topAnchor.constraint(equalTo: topAnchor, constant: 21),
            iconView.widthAnchor.constraint(equalToConstant: 32), iconView.heightAnchor.constraint(equalToConstant: 32),
            titleLabel.topAnchor.constraint(equalTo: iconView.bottomAnchor, constant: 14), titleLabel.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 5), titleLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -5)
        ])
    }
    override func hitTest(_ point: NSPoint) -> NSView? { bounds.contains(convert(point, from: superview)) ? self : nil }
    override func draw(_ dirtyRect: NSRect) {
        let fill: NSColor
        switch feedback {
        case .hover: fill = NSColor(srgbRed: 41.0 / 255, green: 41.0 / 255, blue: 41.0 / 255, alpha: 1)
        case .pressed: fill = ShellPalette.top
        case .selected: fill = NSColor(srgbRed: 0.12, green: 0.2, blue: 0.3, alpha: 1)
        case .normal, .disabled: fill = ShellPalette.card
        }
        fill.setFill()
        NSBezierPath(roundedRect: bounds, xRadius: 18, yRadius: 18).fill()
    }
}
private final class DashboardActionButton: HoverFeedbackButton {
    enum Style: String { case bluePill = "blue-pill", neutralPill = "neutral-pill", blueLink = "blue-link" }
    let style: Style
    init(title: String, style: Style, target: AnyObject?, action: Selector) {
        self.style = style; super.init(frame: .zero)
        self.title = title; self.target = target; self.action = action; isBordered = false
        font = .systemFont(ofSize: style == .blueLink ? 14 : 13, weight: style == .bluePill ? .semibold : .regular)
        setAccessibilityLabel(title); setAccessibilityRole(.button)
        toolTip = title; setAccessibilityHelp(title)
    }
    required init?(coder: NSCoder) { fatalError() }
    override var feedbackCornerRadius: CGFloat { style == .blueLink ? 5 : bounds.height / 2 }
    override func draw(_ dirtyRect: NSRect) {
        if style != .blueLink {
            let normal = style == .bluePill ? ShellPalette.accent : NSColor(calibratedWhite: 0.18, alpha: 1)
            let fill: NSColor
            switch feedback {
            case .hover: fill = normal.blended(withFraction: 0.12, of: .white) ?? normal
            case .pressed: fill = normal.blended(withFraction: 0.22, of: .black) ?? normal
            case .selected: fill = normal.blended(withFraction: 0.12, of: .black) ?? normal
            case .disabled: fill = NSColor(calibratedWhite: 0.16, alpha: 1)
            case .normal: fill = normal
            }
            fill.setFill()
            NSBezierPath(roundedRect: bounds, xRadius: bounds.height / 2, yRadius: bounds.height / 2).fill()
        }
        var color = style == .blueLink ? ShellPalette.accent : ShellPalette.primary
        if !isEnabled { color = ShellPalette.secondary }
        else if style == .blueLink {
            if feedback == .hover { color = color.blended(withFraction: 0.25, of: .white) ?? color }
            else if feedback == .pressed { color = color.blended(withFraction: 0.22, of: .black) ?? color }
        }
        let attrs: [NSAttributedString.Key: Any] = [.font: font ?? NSFont.systemFont(ofSize: 13), .foregroundColor: color]
        let size = (title as NSString).size(withAttributes: attrs)
        (title as NSString).draw(at: NSPoint(x: (bounds.width-size.width)/2, y: (bounds.height-size.height)/2), withAttributes: attrs)
    }
}

private final class HomeDropZoneView: NSView {
    let usesDashedBorder = true
    private var hoverTrackingArea: NSTrackingArea?
    private var isPointerInside = false
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let hoverTrackingArea { removeTrackingArea(hoverTrackingArea) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self, userInfo: nil)
        addTrackingArea(area); hoverTrackingArea = area
    }
    override func mouseEntered(with event: NSEvent) { setPointerInside(true) }
    override func mouseExited(with event: NSEvent) { setPointerInside(false) }
    private func setPointerInside(_ inside: Bool) {
        isPointerInside = inside; needsDisplay = true
        subviews.compactMap { $0 as? HomePDFStackGlyph }.forEach { $0.isEmphasized = inside }
    }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { setPointerInside(false) }
    }
    override func draw(_ dirtyRect: NSRect) {
        (isPointerInside ? NSColor(srgbRed: 0.13, green: 0.14, blue: 0.16, alpha: 1) : ShellPalette.card).setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 1, dy: 1), xRadius: 18, yRadius: 18).fill()
        (isPointerInside ? ShellPalette.accent.withAlphaComponent(0.55) : NSColor(calibratedWhite: 0.19, alpha: 1)).setStroke()
        let edge = NSBezierPath(roundedRect: bounds.insetBy(dx: 1, dy: 1), xRadius: 18, yRadius: 18); edge.lineWidth = 1.5; edge.stroke()
        let dash = NSBezierPath(roundedRect: bounds.insetBy(dx: 7, dy: 7), xRadius: 12, yRadius: 12); dash.lineWidth = 1.5
        dash.setLineDash([10, 7], count: 2, phase: 0); dash.stroke()
    }
}

private final class HomePDFStackGlyph: NSView {
    var isEmphasized = false { didSet { needsDisplay = true } }
    override func draw(_ dirtyRect: NSRect) {
        let ink = isEmphasized ? NSColor(calibratedWhite: 0.43, alpha: 1) : NSColor(calibratedWhite: 0.27, alpha: 1)
        let paper = NSColor(calibratedWhite: isEmphasized ? 0.23 : 0.19, alpha: 1)
        for angle: CGFloat in [-15, 15] {
            NSGraphicsContext.saveGraphicsState()
            let transform = AffineTransform(translationByX: bounds.midX + (angle < 0 ? -12 : 12), byY: bounds.midY - 3)
            var turned = transform; turned.rotate(byDegrees: angle); (turned as NSAffineTransform).concat()
            let rect = NSRect(x: -21, y: -27, width: 42, height: 48)
            paper.setFill(); let back = NSBezierPath(roundedRect: rect, xRadius: 1, yRadius: 1); back.fill(); ink.setStroke(); back.lineWidth = 0.8; back.stroke()
            for y: CGFloat in [12, 8, 4, 0, -4] { let line = NSBezierPath(); line.move(to: NSPoint(x: -17, y: y)); line.line(to: NSPoint(x: 17, y: y)); line.lineWidth = 0.8; line.stroke() }
            NSGraphicsContext.restoreGraphicsState()
        }
        let front = NSRect(x: bounds.midX - 22, y: 3, width: 44, height: 60)
        NSColor(calibratedWhite: isEmphasized ? 0.25 : 0.2, alpha: 1).setFill(); let outline = NSBezierPath(roundedRect: front, xRadius: 2, yRadius: 2); outline.fill(); ink.setStroke(); outline.lineWidth = 1; outline.stroke()
        let attrs: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: 14, weight: .bold), .foregroundColor: NSColor(calibratedWhite: 0.4, alpha: 1)]
        let size = ("PDF" as NSString).size(withAttributes: attrs)
        ("PDF" as NSString).draw(at: NSPoint(x: bounds.midX - size.width/2, y: 11), withAttributes: attrs)
    }
}

private final class HomeFolderIllustration: NSView {
    let kind: String
    init(kind: String) { self.kind = kind; super.init(frame: .zero) }
    required init?(coder: NSCoder) { fatalError() }
    override func draw(_ dirtyRect: NSRect) {
        NSColor(calibratedWhite: 0.19, alpha: 1).setStroke()
        let folder = NSBezierPath(); folder.lineWidth = 2.8; folder.lineCapStyle = .round; folder.lineJoinStyle = .round
        folder.move(to: NSPoint(x: 10, y: 5)); folder.line(to: NSPoint(x: 7, y: 44)); folder.curve(to: NSPoint(x: 12, y: 49), controlPoint1: NSPoint(x: 7, y: 48), controlPoint2: NSPoint(x: 8, y: 49))
        folder.line(to: NSPoint(x: 56, y: 49)); folder.curve(to: NSPoint(x: 61, y: 44), controlPoint1: NSPoint(x: 61, y: 49), controlPoint2: NSPoint(x: 61, y: 48)); folder.line(to: NSPoint(x: 61, y: 38))
        if kind == "recent-folder" {
            folder.move(to: NSPoint(x: 10, y: 5)); folder.line(to: NSPoint(x: 20, y: 37)); folder.curve(to: NSPoint(x: 25, y: 40), controlPoint1: NSPoint(x: 21, y: 40), controlPoint2: NSPoint(x: 22, y: 40)); folder.line(to: NSPoint(x: 72, y: 40)); folder.line(to: NSPoint(x: 61, y: 5))
        } else {
            folder.line(to: NSPoint(x: 61, y: 5)); folder.move(to: NSPoint(x: 10, y: 5)); folder.line(to: NSPoint(x: 10, y: 5))
            let star = NSBezierPath(); let center = NSPoint(x: 50, y: 32)
            for index in 0..<10 { let radius: CGFloat = index.isMultiple(of: 2) ? 8 : 3.5; let angle = CGFloat(index) * .pi / 5 + .pi / 2; let point = NSPoint(x: center.x + cos(angle)*radius, y: center.y + sin(angle)*radius); if index == 0 { star.move(to: point) } else { star.line(to: point) } }; star.close(); NSColor(calibratedWhite: 0.19, alpha: 1).setFill(); star.fill()
        }
        folder.stroke()
    }
}

private final class PDFDropView: NSView {
    var onFiles: (([URL]) -> Bool)?
    var onResize: (() -> Void)?
    override func layout() { super.layout(); onResize?() }
    override init(frame frameRect: NSRect) { super.init(frame: frameRect); registerForDraggedTypes([.fileURL]) }
    required init?(coder: NSCoder) { super.init(coder: coder); registerForDraggedTypes([.fileURL]) }
    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard onFiles != nil, sender.draggingSourceOperationMask.contains(.copy), PDFFileDrop.urls(from: sender.draggingPasteboard) != nil else { return [] }; return .copy
    }
    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation { draggingEntered(sender) }
    override func prepareForDragOperation(_ sender: NSDraggingInfo) -> Bool { draggingEntered(sender) == .copy }
    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        guard draggingEntered(sender) == .copy, let urls = PDFFileDrop.urls(from: sender.draggingPasteboard) else { return false }; return onFiles?(urls) ?? false
    }
}
private final class FlippedDashboardView: NSView { override var isFlipped: Bool { true } }
private extension NSView { func addSubviewPinned(_ child: NSView, inset: CGFloat) { child.translatesAutoresizingMaskIntoConstraints = false; addSubview(child); NSLayoutConstraint.activate([child.leadingAnchor.constraint(equalTo: leadingAnchor, constant: inset), child.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -inset), child.topAnchor.constraint(equalTo: topAnchor, constant: inset), child.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -inset)]) } }
