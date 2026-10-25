import AppKit
import PDFKit
import PaperCore
import UniformTypeIdentifiers
import ObjectiveC.runtime

private final class AnnotationPresentationBacking: NSObject {
    static var associationKey: UInt8 = 0
    let bytes: Data
    let document: CGPDFDocument?
    init(bytes: Data, source: PDFDocument) { self.bytes = bytes; self.document = source.documentRef }
}

enum PendingPlacement {
    case note(NoteConfiguration)
    case freeText(String)
    case signature([[CGPoint]])
    case shape(AnnotationShape, AnnotationConfiguration)
    case line(LineEndings, AnnotationConfiguration)

    func bounds(from start: CGPoint, to end: CGPoint, within crop: CGRect, rotation: Int = 0) throws -> CGRect {
        guard [start.x, start.y, end.x, end.y, crop.minX, crop.minY, crop.width, crop.height].allSatisfy(\.isFinite), crop.width > 0, crop.height > 0 else {
            throw PDFWorkspaceError.invalidSelection
        }
        let size: CGSize
        switch self {
        case .note: size = CGSize(width: 24, height: 24)
        case .freeText: size = CGSize(width: 240, height: 72)
        case .signature:
            size = rotation % 180 == 0 ? CGSize(width: 160, height: 70) : CGSize(width: 70, height: 160)
        case .shape, .line:
            return try ToolInputValidation.normalizedDrag(from: start, to: end, within: crop)
        }
        if start != end {
            return try ToolInputValidation.normalizedDrag(from: start, to: end, within: crop)
        }
        let fit = min(1, crop.width / size.width, crop.height / size.height)
        let width = size.width * fit, height = size.height * fit
        return CGRect(x: min(max(start.x-width/2, crop.minX), crop.maxX-width),
                      y: min(max(start.y-height/2, crop.minY), crop.maxY-height),
                      width: width, height: height)
    }

    static func signatureStrokes(_ strokes: [[CGPoint]], in bounds: CGRect, rotation: Int) -> [[CGPoint]] {
        let rotation = ((rotation % 360) + 360) % 360
        return strokes.map { stroke in stroke.map { point in
            let p: CGPoint
            switch rotation {
            case 90: p = CGPoint(x: 1-point.y, y: point.x)
            case 180: p = CGPoint(x: 1-point.x, y: 1-point.y)
            case 270: p = CGPoint(x: point.y, y: 1-point.x)
            default: p = point
            }
            return CGPoint(x: bounds.minX+p.x*bounds.width, y: bounds.minY+p.y*bounds.height)
        } }
    }
}

enum WorkbenchMode: Equatable { case annotation, editing, drawing, signature, pages, export }
private extension WorkbenchMode {
    var statusTitle: String { switch self { case .annotation: return "批注"; case .editing: return "编辑"; case .drawing: return "绘图"; case .signature: return "填写和签名"; case .pages: return "页面管理"; case .export: return "导出" } }
}

@MainActor
final class WorkbenchController: NSViewController, NSTextFieldDelegate, NSSearchFieldDelegate, NSMenuItemValidation {
    let workspace = PDFWorkspace()
    var onShowCopilot: (() -> Void)?
    var onDocumentSave: (() -> Void)?
    var onDocumentRevisionChange: (() -> Void)?
    private var lastCopilotRevision: Int?
    private func invalidateCopilotIfChanged() {
        if lastCopilotRevision != workspace.revision {
            lastCopilotRevision = workspace.revision; onDocumentRevisionChange?()
        }
    }
    var copilotPageIndex: Int { currentPageIndex }
    private let copilotHost = NSView()
    private var copilotClosedWidth: NSLayoutConstraint?
    private var copilotOpenWidths: [NSLayoutConstraint] = []
    var isCopilotVisible: Bool { !copilotHost.subviews.isEmpty }
    func setCopilotPanel(_ panel: NSView?) {
        _ = view
        copilotHost.subviews.forEach { $0.removeFromSuperview() }
        NSLayoutConstraint.deactivate(copilotOpenWidths); copilotClosedWidth?.isActive = false
        if let panel {
            copilotHost.isHidden = false; NSLayoutConstraint.activate(copilotOpenWidths)
            panel.translatesAutoresizingMaskIntoConstraints = false; copilotHost.addSubview(panel)
            NSLayoutConstraint.activate([panel.leadingAnchor.constraint(equalTo: copilotHost.leadingAnchor), panel.trailingAnchor.constraint(equalTo: copilotHost.trailingAnchor), panel.topAnchor.constraint(equalTo: copilotHost.topAnchor), panel.bottomAnchor.constraint(equalTo: copilotHost.bottomAnchor)])
        } else { copilotHost.isHidden = true; copilotClosedWidth?.isActive = true }
        view.layoutSubtreeIfNeeded()
    }
    func navigateCopilot(to index: Int) {
        guard index >= 0, index < (workspace.document?.pageCount ?? 0) else { return }
        if isOverviewVisible { setOverviewVisible(false) }
        try? readerSession.goToPage(index + 1)
    }
    @objc private func openCopilot() { onShowCopilot?() }
    var localToolCancellation: LocalToolCancellation?
    var compressionPanelController: PDFCompressionPanelController?
    var isLocalToolRunning: Bool { localToolCancellation != nil }
    var isContentEditPending = false
    var contentToolID: String?
    let pdfView = InkPDFView()
    lazy var readerSession = ReaderSession(pdfView: pdfView)
    lazy var readerSidebar = ReaderSidebarController(session: readerSession)
    lazy var readerCommands = ReaderCommandController(session: readerSession, sidebar: readerSidebar)
    let pageOverview = PageOverviewController()
    private(set) var isOverviewVisible = false
    private var readerPresentationNeedsRefresh = false
    private let emptyView = NSStackView()
    private let pageField = NSTextField(string: "1")
    private let pageCount = NSTextField(labelWithString: "/ 0")
    private let searchField = NSSearchField()
    private let searchStatus = NSTextField(labelWithString: "输入关键词搜索")
    private let status = NSTextField(labelWithString: "PDF Master · 本地阅读与批注")
    private let fileImportCancelButton = NSButton(title: "取消导入", target: nil, action: nil)
    func setFileImportProgress(_ running: Bool) { fileImportCancelButton.isHidden = !running; fileImportCancelButton.isEnabled = running }
    @objc private func cancelFileImportFromButton() {
        cancelLocalTool(); fileImportCancelButton.isEnabled = false
        setInteractionStatus("正在取消导入… 不会插入未完成的文件")
    }
    private let pageBadge = NSTextField(labelWithString: "0 / 0")
    private lazy var zoomPopup = readerCommands.makeZoomPopup()
    private var sidebarWidthConstraint: NSLayoutConstraint?
    private weak var sidebarScrollView: NSView?
    private var toolbarControls: [NSControl] = []
    private var toolbarGroups: [NSView] = []
    var toolbarGroupFrames: [NSRect] { toolbarGroups.map { $0.convert($0.bounds, to: view) } }
    private let annotationPalette = NSView()
    private let secondaryTools = NSStackView()
    private let secondaryToolRow = NSStackView()
    let textStyleToolbarHost = NSView()
    var secondaryToolIDs: [String] { secondaryTools.arrangedSubviews.compactMap { ($0 as? ToolStripButton)?.toolID } }
    var secondaryToolFrame: NSRect { secondaryTools.convert(secondaryTools.bounds, to: view) }
    private var primaryModeButtons: [ModeButton] = []
    private let layoutPopover = NSPopover()
    private var eraserEnabled = false
    var clipboard = DocumentClipboard.shared
    private var paletteHeightConstraint: NSLayoutConstraint?
    private(set) var activeMode: WorkbenchMode?
    private var documentControls: [NSControl] = []
    private var copyingControls: [NSControl] = []
    private var undoButton: NSButton?
    private var redoButton: NSButton?
    private weak var inkMenuItem: NSMenuItem?
    private let annotationSelection = AnnotationSelectionTracker()
    private var pendingPlacement: PendingPlacement?
    private var activeMarkup: (kind: PDFAnnotationSubtype, color: NSColor)?
    let shapeStyleBar = ShapeStyleBarView()
    /// A separate instance keeps pen width/color independent of shape defaults.
    let inkStyleBar = ShapeStyleBarView()
    private var drawingInkStyle = AnnotationStyle(strokeColor: .systemIndigo, fillColor: nil, lineWidth: 2)
    private var selectedShape: (page: Int, index: Int, revision: Int)?
    private var selectedInk: (page: Int, index: Int, revision: Int, document: ObjectIdentifier)?
    private var drawingShapeStyle = AnnotationStyle(strokeColor: .systemRed, fillColor: nil, lineWidth: 2)
    private var sourceProtection: SourceProtection?
    var interactionStatusText: String { status.stringValue }
    var interactionStatusToolTip: String? { status.toolTip }
    let workbenchToolbarHeight: CGFloat = 46
    let thumbnailSidebarWidth: CGFloat = 176
    var pageBadgeText: String { pageBadge.stringValue }
    var thumbnailSidebarFrame: NSRect { sidebarScrollView?.frame ?? .zero }
    var toolbarAccessibilityLabels: [String] { toolbarControls.compactMap { $0.accessibilityLabel() } }

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 1280, height: 820)); root.autoresizingMask = [.width, .height]; root.wantsLayer = true; root.layer?.backgroundColor = ShellPalette.canvas.cgColor
        view = root
        let toolbar = makeToolbar()
        configureAnnotationPalette()
        let content = NSView()
        let side = NSView(); sidebarScrollView = side
        side.wantsLayer = true; side.layer?.backgroundColor = ShellPalette.sidebar.cgColor
        addChild(readerSidebar)
        let readerSideView = readerSidebar.view
        readerSideView.translatesAutoresizingMaskIntoConstraints = false; side.addSubview(readerSideView)
        side.layer?.masksToBounds = true
        let sideTrailing = readerSideView.trailingAnchor.constraint(equalTo: side.trailingAnchor)
        sideTrailing.priority = .defaultHigh
        NSLayoutConstraint.activate([
            readerSideView.leadingAnchor.constraint(equalTo: side.leadingAnchor),
            sideTrailing,
            readerSideView.widthAnchor.constraint(greaterThanOrEqualToConstant: thumbnailSidebarWidth),
            readerSideView.topAnchor.constraint(equalTo: side.topAnchor),
            readerSideView.bottomAnchor.constraint(equalTo: side.bottomAnchor)
        ])
        readerSession.onChange = { [weak self] in
            guard let self else { return }
            self.readerSidebar.refresh()
            self.searchStatus.stringValue = self.readerSession.searchStatus
        }
        readerSidebar.onAnnotationSelection = { [weak self] page, index in
            guard let self else { return }
            self.resetDocumentInteractionState(); self.eraserEnabled = false
            self.setOverviewVisible(false)
            if let target = self.pdfView.document?.page(at: page) { self.pdfView.go(to: target) }
            self.selectAnnotation(page: page, index: index)
        }
        readerSidebar.onAnnotationTool = { [weak self] id in
            guard let self else { return }
            self.enterMode(.annotation)
            if let button = self.secondaryTools.arrangedSubviews.compactMap({ $0 as? ToolStripButton }).first(where: { $0.toolID == id }) {
                self.runSecondaryTool(button)
            }
        }
        readerSidebar.canDeleteAnnotation = { [weak self] _, _ in self?.workspace.allowsCommenting == true }
        readerSidebar.onDeleteAnnotation = { [weak self] page, index in
            guard let self else { return }
            self.selectAnnotation(page: page, index: index)
            self.deleteSelectedAnnotation()
        }
        readerCommands.setSidebarVisible = { [weak self] visible in
            guard let self else { return }
            self.sidebarWidthConstraint?.constant = visible ? self.thumbnailSidebarWidth : 0
            self.sidebarScrollView?.isHidden = !visible
        }
        readerCommands.onError = { [weak self] error in self?.setInteractionStatus("阅读操作失败：\(error)") }
        readerCommands.interceptCommand = { [weak self] command in
            guard let self, self.isOverviewVisible else { return nil }
            switch command {
            case .zoomIn: self.pageOverview.setThumbnailScale(self.pageOverview.thumbnailScale * 1.2); return true
            case .zoomOut: self.pageOverview.setThumbnailScale(self.pageOverview.thumbnailScale / 1.2); return true
            case .fitPage, .fitWidth: self.pageOverview.setThumbnailScale(1); return true
            case .actualSize, .singlePage, .twoPages, .cover: return false
            case .outline, .thumbnails, .toggleSidebar: self.setOverviewVisible(false); return nil
            default: return nil
            }
        }
        pdfView.autoScales = true; pdfView.displayMode = .singlePageContinuous; pdfView.displaysPageBreaks = true
        pdfView.backgroundColor = ShellPalette.canvas
        pdfView.setAccessibilityLabel("PDF 文档视图")
        side.translatesAutoresizingMaskIntoConstraints = false; pdfView.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(side); content.addSubview(pdfView)
        let sideWidth = side.widthAnchor.constraint(equalToConstant: thumbnailSidebarWidth); sidebarWidthConstraint = sideWidth
        NSLayoutConstraint.activate([
            side.leadingAnchor.constraint(equalTo: content.leadingAnchor), side.topAnchor.constraint(equalTo: content.topAnchor), side.bottomAnchor.constraint(equalTo: content.bottomAnchor), sideWidth,
            pdfView.leadingAnchor.constraint(equalTo: side.trailingAnchor), pdfView.trailingAnchor.constraint(equalTo: content.trailingAnchor), pdfView.topAnchor.constraint(equalTo: content.topAnchor), pdfView.bottomAnchor.constraint(equalTo: content.bottomAnchor)
        ])
        addChild(pageOverview)
        let overviewView = pageOverview.view
        overviewView.translatesAutoresizingMaskIntoConstraints = false; content.addSubview(overviewView); overviewView.isHidden = true
        NSLayoutConstraint.activate([overviewView.leadingAnchor.constraint(equalTo: content.leadingAnchor), overviewView.trailingAnchor.constraint(equalTo: content.trailingAnchor), overviewView.topAnchor.constraint(equalTo: content.topAnchor), overviewView.bottomAnchor.constraint(equalTo: content.bottomAnchor)])
        pageOverview.onOpenPage = { [weak self] index in
            guard let self else { return }
            self.setOverviewVisible(false)
            if let page = self.pdfView.document?.page(at: index) { self.pdfView.go(to: page) }
        }
        pageOverview.onSelectionChange = { [weak self] _ in self?.pageChanged() }
        pageOverview.onDropPDFs = { [weak self] urls, index in self?.queueFileMerge(urls, at: index) ?? false }
        pageOverview.onMovePages = { [weak self] pages, insertion in
            guard let self, self.isOverviewVisible else { return false }
            do {
                try self.workspace.movePages(pages: pages, to: insertion)
                let start = insertion - pages.count(in: 0..<insertion)
                self.refreshDocument()
                self.pageOverview.restoreSelection(IndexSet(integersIn: start..<(start + pages.count)))
                self.view.window?.makeFirstResponder(self.pageOverview.collectionView)
                self.setInteractionStatus("已移动 \(pages.count) 页，可撤销")
                return true
            } catch let error as PDFWorkspaceError { self.showError(WorkbenchMessage.error(error)); return false }
            catch { self.showError("移动页面失败：\(error.localizedDescription)"); return false }
        }
        pageOverview.onDeleteSelection = { [weak self] in
            guard let self, self.isOverviewVisible else { return }
            self.perform { try self.deleteSelectedPages(self.pageOverview.selectedPages) }
        }
        readerSidebar.onDeletePages = { [weak self] pages in
            guard let self, !self.isOverviewVisible else { return }
            self.perform { try self.deleteSelectedPages(pages) }
        }
        pageOverview.collectionView.onClipboard = { [weak self] command in self?.runPageClipboard(command, overview: true) }
        pageOverview.collectionView.canClipboard = { [weak self] command in self?.canPageClipboard(command, overview: true) ?? false }
        readerSidebar.thumbnailView.onClipboard = { [weak self] command in self?.runPageClipboard(command, overview: false) }
        readerSidebar.thumbnailView.canClipboard = { [weak self] command in self?.canPageClipboard(command, overview: false) ?? false }
        pdfView.onClipboard = { [weak self] command in self?.runAnnotationClipboard(command) ?? false }
        pdfView.canClipboard = { [weak self] command in self?.canAnnotationClipboard(command) ?? false }
        pdfView.onDropFiles = { [weak self] urls in
            guard let self else { return false }
            return self.queueFileMerge(urls, at: self.currentPageIndex + 1)
        }
        status.font = .systemFont(ofSize: 11); status.textColor = ShellPalette.secondary; status.alignment = .center; status.wantsLayer = true; status.layer?.backgroundColor = ShellPalette.canvas.cgColor
        pageBadge.font = .monospacedDigitSystemFont(ofSize: 11, weight: .medium); pageBadge.textColor = ShellPalette.primary; pageBadge.alignment = .center; pageBadge.wantsLayer = true; pageBadge.layer?.backgroundColor = ShellPalette.top.cgColor; pageBadge.layer?.cornerRadius = 7
        [toolbar, annotationPalette, content, copilotHost, status, pageBadge].forEach { $0.translatesAutoresizingMaskIntoConstraints = false; root.addSubview($0) }
        copilotHost.isHidden = true
        copilotClosedWidth = copilotHost.widthAnchor.constraint(equalToConstant: 0); copilotClosedWidth?.isActive = true
        let proportional = copilotHost.widthAnchor.constraint(equalTo: root.widthAnchor, multiplier: 0.28); proportional.priority = .defaultHigh
        copilotOpenWidths = [proportional, copilotHost.widthAnchor.constraint(greaterThanOrEqualToConstant: 340), copilotHost.widthAnchor.constraint(lessThanOrEqualToConstant: 560)]
        fileImportCancelButton.identifier = NSUserInterfaceItemIdentifier("document-file-import-cancel")
        fileImportCancelButton.target = self; fileImportCancelButton.action = #selector(cancelFileImportFromButton)
        fileImportCancelButton.bezelStyle = .inline; fileImportCancelButton.isHidden = true
        fileImportCancelButton.setAccessibilityLabel("取消文件导入")
        fileImportCancelButton.translatesAutoresizingMaskIntoConstraints = false; root.addSubview(fileImportCancelButton)
        NSLayoutConstraint.activate([fileImportCancelButton.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -8), fileImportCancelButton.centerYAnchor.constraint(equalTo: status.centerYAnchor), fileImportCancelButton.widthAnchor.constraint(equalToConstant: 80)])
        let paletteHeight = annotationPalette.heightAnchor.constraint(equalToConstant: 0); paletteHeightConstraint = paletteHeight
        NSLayoutConstraint.activate([
            toolbar.topAnchor.constraint(equalTo: root.topAnchor), toolbar.leadingAnchor.constraint(equalTo: root.leadingAnchor), toolbar.trailingAnchor.constraint(equalTo: root.trailingAnchor), toolbar.heightAnchor.constraint(equalToConstant: workbenchToolbarHeight),
            annotationPalette.topAnchor.constraint(equalTo: toolbar.bottomAnchor), annotationPalette.leadingAnchor.constraint(equalTo: root.leadingAnchor), annotationPalette.trailingAnchor.constraint(equalTo: root.trailingAnchor), paletteHeight,
            content.topAnchor.constraint(equalTo: annotationPalette.bottomAnchor), content.leadingAnchor.constraint(equalTo: root.leadingAnchor), content.trailingAnchor.constraint(equalTo: copilotHost.leadingAnchor), content.bottomAnchor.constraint(equalTo: status.topAnchor),
            copilotHost.topAnchor.constraint(equalTo: content.topAnchor), copilotHost.bottomAnchor.constraint(equalTo: content.bottomAnchor), copilotHost.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            status.leadingAnchor.constraint(equalTo: root.leadingAnchor), status.trailingAnchor.constraint(equalTo: root.trailingAnchor), status.bottomAnchor.constraint(equalTo: root.bottomAnchor), status.heightAnchor.constraint(equalToConstant: 25),
            pageBadge.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -16), pageBadge.bottomAnchor.constraint(equalTo: status.topAnchor, constant: -12), pageBadge.widthAnchor.constraint(greaterThanOrEqualToConstant: 58), pageBadge.heightAnchor.constraint(equalToConstant: 26)
        ])
        configureEmptyView(in: root)
        NotificationCenter.default.addObserver(self, selector: #selector(pageChanged), name: .PDFViewPageChanged, object: pdfView)
        NotificationCenter.default.addObserver(self, selector: #selector(pageChanged), name: .PDFViewScaleChanged, object: pdfView)
        pdfView.onInk = { [weak self] points, page, style in
            self?.commitInk(points: points, page: page, style: style)
        }
        pdfView.onPlacement = { [weak self] start, end, page in self?.applyPlacement(from: start, to: end, page: page) }
        pdfView.onTextSelectionCompleted = { [weak self] selection in
            guard let self, let tool = self.activeMarkup else { return }
            self.applyMarkup(selection, kind: tool.kind, color: tool.color)
        }
        pdfView.onAnnotationSelection = { [weak self] page, index in self?.selectAnnotation(page: page, index: index) }
        pdfView.onTextAnnotationEditing = { [weak self] page, index in self?.editFreeTextInline(page: page, annotationIndex: index) }
        pdfView.onShapeTransform = { [weak self] page, index, bounds in
            guard let self, let a = self.workspace.document?.page(at: page)?.annotations[safe: index] else { return }
            if a.type == "FreeText" {
                self.mutateAnnotations(pages: IndexSet(integer: page)) {
                    try self.workspace.updateFreeText(page: page, index: index, text: a.contents ?? "", bounds: bounds,
                        font: a.font ?? .systemFont(ofSize: 12), color: a.fontColor ?? .black,
                        rotation: (a.value(forAnnotationKey: PDFAnnotationKey(rawValue: "/Rotate")) as? Int) ?? 0)
                }
                self.selectAnnotation(page: page, index: index); return
            }
            let style = AnnotationStyle(annotation: a)
            self.mutateAnnotations(pages: IndexSet(integer: page)) { try self.workspace.updateShape(page: page, index: index, bounds: bounds, style: style) }
            self.selectAnnotation(page: page, index: index)
        }
        pdfView.canDeleteAnnotation = { [weak self] in self?.canDeleteSelectedAnnotation == true }
        pdfView.onDeleteShape = { [weak self] in self?.deleteSelectedAnnotation() }
        pdfView.onShapeDeselection = { [weak self] in
            guard let self else { return }
            self.selectedShape = nil; self.selectedInk = nil; self.annotationSelection.clear()
            if !self.pdfView.inkEnabled { self.inkStyleBar.isHidden = true }
            if let tool = self.pdfView.shapeTool { self.shapeStyleBar.configure(tool: tool, style: self.drawingShapeStyle) }
            else { self.shapeStyleBar.isHidden = true; if self.activeMode != nil { self.paletteHeightConstraint?.constant = self.pdfView.inkEnabled ? 84 : 34 } }
        }
        pdfView.onCancelPlacement = { [weak self] in self?.cancelPendingPlacement() }
        pdfView.onBlockedWidget = { [weak self] in self?.showError("暂不支持编辑交互式表单字段。文档未修改。") }
        pdfView.onCopyDenied = { [weak self] in self?.showError("文档的原始权限不允许复制或剪切内容。") }
        refreshDocument()
    }

    private func makeToolbar() -> NSView {
        let bar = NSView(); bar.wantsLayer = true; bar.layer?.backgroundColor = ShellPalette.top.cgColor
        let left = NSStackView(), center = NSStackView(), right = NSStackView()
        toolbarGroups = [left, center, right]
        for group in [left, center, right] {
            group.orientation = .horizontal; group.spacing = 8; group.alignment = .centerY
            group.translatesAutoresizingMaskIntoConstraints = false; bar.addSubview(group)
            group.centerYAnchor.constraint(equalTo: bar.centerYAnchor).isActive = true
        }
        func button(_ title: String, _ action: Selector, symbol: String? = nil, enabled: Bool = true) -> NSButton {
            let b = NSButton(title: title, target: self, action: action)
            b.bezelStyle = .inline; b.isBordered = false; b.font = .systemFont(ofSize: 12); b.contentTintColor = ShellPalette.primary
            b.setAccessibilityLabel(title); b.isEnabled = enabled
            if let symbol {
                b.image = NSImage(systemSymbolName: symbol, accessibilityDescription: title)
                b.imagePosition = .imageOnly; b.widthAnchor.constraint(equalToConstant: 24).isActive = true
            }
            if enabled { documentControls.append(b) }
            else { b.toolTip = "暂不支持此功能" }
            toolbarControls.append(b); return b
        }
        left.addArrangedSubview(button("侧栏", #selector(toggleSidebar), symbol: "sidebar.left"))
        left.addArrangedSubview(button("页面预览", #selector(togglePageOverview), symbol: "square.grid.2x2"))
        left.addArrangedSubview(button("布局选项", #selector(showLayoutOptions(_:)), symbol: "book"))
        zoomPopup.font = .systemFont(ofSize: 11)
        zoomPopup.widthAnchor.constraint(equalToConstant: 80).isActive = true
        left.addArrangedSubview(zoomPopup)
        left.addArrangedSubview(button("缩小", #selector(zoomOut), symbol: "minus"))
        left.addArrangedSubview(button("放大", #selector(zoomIn), symbol: "plus"))
        left.addArrangedSubview(button("适合页面", #selector(fit), symbol: "arrow.up.left.and.arrow.down.right"))
        for (title, mode) in [("批注", WorkbenchMode.annotation), ("编辑", WorkbenchMode.editing), ("导出", WorkbenchMode.export), ("填写和签名", WorkbenchMode.signature), ("扫描和 OCR", nil), ("度量", nil)] {
            let b = ModeButton(title: title, target: self, action: #selector(selectMode(_:)))
            b.mode = mode; b.bezelStyle = .inline; b.isBordered = false; b.font = .systemFont(ofSize: 12)
            b.contentTintColor = ShellPalette.primary; b.isEnabled = mode != nil
            b.toolTip = mode == nil ? "暂不支持此功能" : nil
            if mode != nil { documentControls.append(b) }
            b.setAccessibilityLabel(title); toolbarControls.append(b); center.addArrangedSubview(b)
            primaryModeButtons.append(b)
            if title == "扫描和 OCR" { b.isEnabled = true; b.action = #selector(openOCR); documentControls.append(b); b.toolTip = "本地文字识别" }
        }
        center.addArrangedSubview(button("更多工具", #selector(showToolPalette), symbol: "plus.circle"))
        right.addArrangedSubview(button("PDF Copilot", #selector(openCopilot), symbol: "sparkles"))
        right.addArrangedSubview(button("翻译", #selector(noop), symbol: "character.book.closed", enabled: false))
        right.addArrangedSubview(button("分享", #selector(noop), symbol: "square.and.arrow.up", enabled: false))
        searchField.placeholderString = "搜索"; searchField.delegate = self
        searchField.setAccessibilityLabel("搜索文档文字"); toolbarControls.append(searchField)
        searchField.widthAnchor.constraint(equalToConstant: 110).isActive = true
        documentControls.append(searchField); copyingControls.append(searchField); right.addArrangedSubview(searchField)
        for (title, selector, symbol) in [("上个结果", #selector(previousMatch), "chevron.left"), ("下个结果", #selector(nextMatch), "chevron.right")] {
            let b = button(title, selector, symbol: symbol); copyingControls.append(b); right.addArrangedSubview(b)
        }
        NSLayoutConstraint.activate([
            left.leadingAnchor.constraint(equalTo: bar.leadingAnchor, constant: 12),
            center.centerXAnchor.constraint(equalTo: bar.centerXAnchor),
            right.trailingAnchor.constraint(equalTo: bar.trailingAnchor, constant: -12),
            left.trailingAnchor.constraint(lessThanOrEqualTo: center.leadingAnchor, constant: -12),
            center.trailingAnchor.constraint(lessThanOrEqualTo: right.leadingAnchor, constant: -12)
        ])
        return bar
    }

    private func configureAnnotationPalette() {
        annotationPalette.wantsLayer = true; annotationPalette.layer?.backgroundColor = ShellPalette.top.cgColor; annotationPalette.isHidden = true
        secondaryTools.orientation = .horizontal; secondaryTools.spacing = 8; secondaryTools.alignment = .centerY
        secondaryToolRow.orientation = .horizontal; secondaryToolRow.spacing = 16; secondaryToolRow.alignment = .centerY
        secondaryToolRow.translatesAutoresizingMaskIntoConstraints = false; annotationPalette.addSubview(secondaryToolRow)
        secondaryToolRow.addArrangedSubview(secondaryTools)
        textStyleToolbarHost.setAccessibilityLabel("顶部文字样式")
        textStyleToolbarHost.isHidden = true
        secondaryToolRow.addArrangedSubview(textStyleToolbarHost)
        NSLayoutConstraint.activate([
            secondaryToolRow.centerXAnchor.constraint(equalTo: annotationPalette.centerXAnchor),
            secondaryToolRow.centerYAnchor.constraint(equalTo: annotationPalette.topAnchor, constant: 17),
            textStyleToolbarHost.widthAnchor.constraint(equalToConstant: 400),
            textStyleToolbarHost.heightAnchor.constraint(equalToConstant: 28)
        ])
        pdfView.textStyleToolbarHost = textStyleToolbarHost
        pdfView.onTextStyleToolbarVisibilityChange = { [weak self] visible in
            guard let self else { return }
            self.textStyleToolbarHost.isHidden = !visible
            if visible {
                self.annotationPalette.isHidden = false
                self.shapeStyleBar.isHidden = true; self.inkStyleBar.isHidden = true
                self.paletteHeightConstraint?.constant = 34
            } else {
                self.annotationPalette.isHidden = self.activeMode == nil
                self.paletteHeightConstraint?.constant = self.activeMode == nil ? 0 :
                    (!self.shapeStyleBar.isHidden || !self.inkStyleBar.isHidden ? 84 : 34)
            }
            self.view.layoutSubtreeIfNeeded()
        }
        shapeStyleBar.translatesAutoresizingMaskIntoConstraints = false; annotationPalette.addSubview(shapeStyleBar); shapeStyleBar.isHidden = true
        NSLayoutConstraint.activate([shapeStyleBar.centerXAnchor.constraint(equalTo: annotationPalette.centerXAnchor), shapeStyleBar.topAnchor.constraint(equalTo: annotationPalette.topAnchor, constant: 36), shapeStyleBar.heightAnchor.constraint(equalToConstant: 42)])
        shapeStyleBar.onToolChange = { [weak self] tool in self?.beginShapeTool(tool) }
        shapeStyleBar.onStylePreview = { [weak self] style in self?.previewShapeStyle(style) }
        shapeStyleBar.onStyleChange = { [weak self] style in self?.changeShapeStyle(style) }
        inkStyleBar.translatesAutoresizingMaskIntoConstraints = false; annotationPalette.addSubview(inkStyleBar); inkStyleBar.isHidden = true
        NSLayoutConstraint.activate([inkStyleBar.centerXAnchor.constraint(equalTo: annotationPalette.centerXAnchor), inkStyleBar.topAnchor.constraint(equalTo: annotationPalette.topAnchor, constant: 36), inkStyleBar.heightAnchor.constraint(equalToConstant: 42)])
        inkStyleBar.configure(tool: .line, style: drawingInkStyle)
        inkStyleBar.toolChooser.isHidden = true
        inkStyleBar.fillEnabledButton.isHidden = true; inkStyleBar.fillColorWell.isHidden = true
        inkStyleBar.setAccessibilityLabel("手绘画笔属性")
        inkStyleBar.strokeColorWell.setAccessibilityLabel("画笔颜色")
        inkStyleBar.onStylePreview = { [weak self] style in self?.previewInkStyle(style) }
        inkStyleBar.onStyleChange = { [weak self] style in self?.changeInkStyle(style) }
    }

    private func configureEmptyView(in root: NSView) {
        emptyView.orientation = .vertical; emptyView.alignment = .centerX; emptyView.spacing = 12
        let title = NSTextField(labelWithString: "PDF Master"); title.font = .systemFont(ofSize: 28, weight: .semibold)
        let copy = NSTextField(wrappingLabelWithString: "在 Mac 上阅读、整理与批注 PDF，让文档与灵感井然有序。") ; copy.alignment = .center; copy.textColor = .secondaryLabelColor
        let actions = NSStackView(); actions.orientation = .horizontal; actions.spacing = 10
        actions.addArrangedSubview(NSButton(title: "打开 PDF…", target: self, action: #selector(openDocument))); actions.addArrangedSubview(NSButton(title: "新建空白 PDF", target: self, action: #selector(newDocument)))
        [title, copy, actions].forEach { emptyView.addArrangedSubview($0) }; emptyView.translatesAutoresizingMaskIntoConstraints = false; root.addSubview(emptyView)
        NSLayoutConstraint.activate([emptyView.centerXAnchor.constraint(equalTo: root.centerXAnchor), emptyView.centerYAnchor.constraint(equalTo: root.centerYAnchor), emptyView.widthAnchor.constraint(lessThanOrEqualToConstant: 460)])
    }

    @discardableResult func open(url: URL) -> Bool {
        do {
            let opened = try DocumentAdmission.admit(
                url: url,
                passwordPrompt: { DocumentToolPanels.passwordForOpening() },
                shouldReplace: { self.guardDirty() },
                replace: { password in
                    if let password { try self.workspace.open(url, password: password) }
                    else { try self.workspace.open(url) }
                }
            )
            if opened {
                sourceProtection = SourceProtection.capture(url)
                resetDocumentInteractionState()
                refreshDocument()
                if let notice = workspace.fontCompatibilityNotice { setInteractionStatus(notice) }
            }
            return opened
        } catch DocumentAdmissionError.unsupportedWidgets {
            showError("暂不支持包含交互式表单的 PDF。文档未打开，当前工作保持不变。")
            return false
        } catch let error as PDFWorkspaceError {
            showError(WorkbenchMessage.error(error))
            return false
        } catch {
            showError("操作失败：\(error.localizedDescription)")
            return false
        }
    }
    @objc func openDocument() { let p = NSOpenPanel(); p.allowedContentTypes = [.pdf]; guard p.runModal() == .OK, let url = p.url else { return }; open(url: url) }
    @objc func newDocument() {
        guard guardDirty() else { return }
        perform {
            try workspace.createBlank()
            sourceProtection = nil
            resetDocumentInteractionState()
            refreshDocument()
        }
    }
    @objc func saveCopy() { _ = saveCopyInteractively() }
    @objc func saveDocument() { _ = saveDocumentInteractively() }
    @objc func saveAsDocument() { _ = saveDocumentInteractively(forceChooseDestination: true) }
    @discardableResult func saveDocumentInteractively(forceChooseDestination: Bool = false, chooseDestination: (() -> URL?)? = nil) -> Bool {
        guard pdfView.finishInlineTextEditing() else { return false }
        guard !isLocalToolRunning, view.window?.attachedSheet == nil else { return false }
        var destination = forceChooseDestination ? nil : workspace.sourceURL
        if destination == nil {
            if let chooseDestination { destination = chooseDestination() }
            else {
                let panel = NSSavePanel(); panel.allowedContentTypes = [.pdf]
                panel.nameFieldStringValue = workspace.sourceURL?.lastPathComponent ?? "未命名.pdf"
                panel.directoryURL = workspace.sourceURL?.deletingLastPathComponent()
                guard panel.runModal() == .OK else { return false }
                destination = panel.url
            }
        }
        guard let destination else { return false }
        do {
            try workspace.save(to: destination)
            captureAdmittedSource(destination)
            refreshDocument(); onDocumentSave?()
            setInteractionStatus("已保存 · \(destination.lastPathComponent)")
            return true
        } catch let error as PDFWorkspaceError { showError(WorkbenchMessage.error(error)); return false }
        catch { showError("保存失败：\(error.localizedDescription)"); return false }
    }
    private func saveCopyInteractively() -> Bool {
        guard pdfView.finishInlineTextEditing() else { return false }
        guard !isLocalToolRunning, view.window?.attachedSheet == nil else { return false }
        let panel = NSSavePanel(); panel.allowedContentTypes = [.pdf]; panel.nameFieldStringValue = "文档副本.pdf"
        guard panel.runModal() == .OK, let url = panel.url else { return false }
        do { try workspace.saveCopy(to: url); refreshDocument(); return true }
        catch PDFWorkspaceError.permissionDenied {
            showError("文档的原始复制权限或保护状态不允许存储普通副本。对于既有加密 PDF，当前版本不能安全地重新导出其保护，也不会回退为明文。")
            return false
        } catch let error as PDFWorkspaceError { showError(WorkbenchMessage.error(error)); return false }
        catch { showError("操作失败：\(error.localizedDescription)"); return false }
    }
    func guardDirty() -> Bool { guard pdfView.finishInlineTextEditing(), !isLocalToolRunning, view.window?.attachedSheet == nil else { return false }; guard workspace.isDirty else { return true }; let alert = NSAlert(); alert.messageText = "保存文档更改？"; alert.informativeText = "关闭前保存更改，或放弃本次修改。"; alert.addButton(withTitle: "保存"); alert.addButton(withTitle: "不保存"); alert.addButton(withTitle: "取消"); switch alert.runModal() { case .alertFirstButtonReturn: return saveDocumentInteractively(); case .alertSecondButtonReturn: return true; default: return false } }

    func refreshDocument(preservePage: Int? = nil, scale: CGFloat? = nil) {
        invalidateCopilotIfChanged()
        if selectedInk != nil, currentSelectedInk == nil { clearSelectedInk() }
        if let s = selectedShape, s.revision != workspace.revision { selectedShape = nil; pdfView.clearShapeSelection() }
        let overviewSelection = pageOverview.selectedPages
        let page = preservePage ?? currentPageIndex
        let hadPresentation = pdfView.document != nil
        let oldScale = scale ?? pdfView.scaleFactor
        let presentationSnapshot = workspace.document?.dataRepresentation()
        let presentation = presentationSnapshot.flatMap { PDFDocument(data: $0) }
        if isOverviewVisible {
            // The reader is hidden. Rebinding PDFKit here rebuilt hundreds of
            // page tiles immediately before the grid could render its first frame.
            readerPresentationNeedsRefresh = true
        } else {
            readerPresentationNeedsRefresh = false
            readerSession.setDocument(presentation, allowsCopying: workspace.allowsCopying, snapshot: presentationSnapshot)
        }
        pdfView.allowsCopying = workspace.document != nil && workspace.allowsCopying
        _ = annotationSelection.current(in: workspace)
        let hasDocument = workspace.document != nil
        documentControls.forEach { $0.isEnabled = hasDocument }
        copyingControls.forEach { $0.isEnabled = hasDocument && workspace.allowsCopying }
        undoButton?.isEnabled = workspace.canUndo
        redoButton?.isEnabled = workspace.canRedo
        if !isOverviewVisible, let document = presentation, document.pageCount > 0 {
            pdfView.isHidden = false; emptyView.isHidden = true
            let target = min(max(page, 0), document.pageCount - 1)
            if let targetPage = document.page(at: target) { pdfView.go(to: targetPage) }
            if hadPresentation || scale != nil { if oldScale > 0 { pdfView.scaleFactor = oldScale } }
            else { pdfView.autoScales = true }
        } else if !isOverviewVisible {
            pdfView.isHidden = true; emptyView.isHidden = false
        }
        pageChanged()
        if isOverviewVisible {
            pageOverview.setDocument(presentation, selectedPage: page, snapshot: presentationSnapshot)
            pageOverview.restoreSelection(overviewSelection)
            pdfView.isHidden = true
        }
        if !isOverviewVisible { refreshSearch() }
        refreshPersistentDocumentStatus()
    }
    private func commitInk(points: [CGPoint], page index: Int, style: AnnotationStyle) {
        perform {
            let presentationPage = self.pdfView.document?.page(at: index)
            let previousCount = presentationPage?.annotations.count
            try self.workspace.addInk(points: points, page: index, color: style.strokeColor,
                                      width: style.lineWidth, smoothed: true)
            guard let presentationPage, let previousCount,
                  let committedPage = self.workspace.document?.page(at: index),
                  committedPage.annotations.count == previousCount + 1,
                  let committed = committedPage.annotations.last, committed.type == "Ink",
                  let paths = committed.paths, !paths.isEmpty else {
                self.refreshDocument(preservePage: index)
                return
            }
            let displayed = PDFAnnotation(bounds: committed.bounds, forType: .ink, withProperties: nil)
            displayed.color = committed.color
            let border = PDFBorder(); border.lineWidth = committed.border?.lineWidth ?? style.lineWidth
            displayed.border = border
            for path in paths { if let copy = path.copy() as? NSBezierPath { displayed.add(copy) } }
            presentationPage.addAnnotation(displayed)
            self.pdfView.refreshCommittedInkRendering()
            self.pdfView.needsDisplay = true
            self.readerSession.didUpdateAnnotations(snapshot: nil)
            self.undoButton?.isEnabled = self.workspace.canUndo
            self.redoButton?.isEnabled = self.workspace.canRedo
            self.refreshPersistentDocumentStatus()
        }
    }
    func mergeDroppedPDFs(_ urls: [URL], at index: Int) {
        perform {
            let before = self.workspace.document?.pageCount ?? 0
            try self.workspace.insertPDFs(urls, at: index)
            let inserted = (self.workspace.document?.pageCount ?? before) - before
            self.refreshDocument(preservePage: index, scale: self.pdfView.scaleFactor)
            if inserted > 0 { self.pageOverview.restoreSelection(IndexSet(integersIn: index..<(index + inserted))) }
            self.setInteractionStatus("已插入 \(urls.count) 份 PDF，共 \(inserted) 页；保存后即可保留更改。")
        }
    }
    private var currentPageIndex: Int { guard let doc = pdfView.document, let page = pdfView.currentPage else { return 0 }; let i = doc.index(for: page); return i == NSNotFound ? 0 : i }
    @objc private func pageChanged() {
        let count = isOverviewVisible ? (workspace.document?.pageCount ?? 0) : (pdfView.document?.pageCount ?? 0)
        let index = isOverviewVisible ? (pageOverview.selectedPages.first ?? 0) : currentPageIndex
        pageField.stringValue = count == 0 ? "—" : "\(index + 1)"
        pageCount.stringValue = "/ \(count)"
        pageBadge.stringValue = count == 0 ? "0 / 0" : "\(index + 1) / \(count)"
        let title = "\(Int((pdfView.scaleFactor * 100).rounded()))%"
        if zoomPopup.item(withTitle: title) == nil { zoomPopup.addItem(withTitle: title) }
        zoomPopup.selectItem(withTitle: title)
    }
    func controlTextDidEndEditing(_ obj: Notification) { if obj.object as? NSTextField === pageField, let doc = pdfView.document, let i = PageJump.index(for: pageField.stringValue, pageCount: doc.pageCount), let p = doc.page(at: i) { pdfView.go(to: p) } else if obj.object as? NSSearchField === searchField { refreshSearch() } }
    func controlTextDidChange(_ obj: Notification) { if obj.object as? NSSearchField === searchField { refreshSearch() } }

    private func refreshSearch() {
        let query = searchField.stringValue
        readerSidebar.searchField.stringValue = query
        if !query.isEmpty { _ = readerCommands.run(.find) }
        do { try readerSession.search(query) }
        catch { searchStatus.stringValue = "搜索不可用：\(error)" }
    }
    @objc private func nextMatch() { readerSession.nextHit() }; @objc private func previousMatch() { readerSession.nextHit(backward: true) }
    @objc private func zoomIn() { if isOverviewVisible { pageOverview.setThumbnailScale(pageOverview.thumbnailScale * 1.2) } else { _ = readerCommands.run(.zoomIn); pageChanged() } }
    @objc private func zoomOut() { if isOverviewVisible { pageOverview.setThumbnailScale(pageOverview.thumbnailScale / 1.2) } else { _ = readerCommands.run(.zoomOut); pageChanged() } }
    @objc private func fit() { if isOverviewVisible { pageOverview.setThumbnailScale(1) } else { _ = readerCommands.run(.fitPage); pageChanged() } }
    @objc func togglePageOverview() {
        if isOverviewVisible { setOverviewVisible(false) }
        else { enterMode(.pages) }
    }
    private func setOverviewVisible(_ visible: Bool) {
        isOverviewVisible = visible
        pageOverview.view.isHidden = !visible; pdfView.isHidden = visible
        sidebarScrollView?.isHidden = visible || !readerCommands.sidebarVisible
        zoomPopup.isEnabled = !visible
        if visible {
            if !readerPresentationNeedsRefresh { pageOverview.setDocument(pdfView.document, selectedPage: currentPageIndex, snapshot: readerSession.documentSnapshot ?? pdfView.presentationDataRepresentation()) }
            pageOverview.refreshLayout(); pageOverview.revealSelection()
            view.window?.makeFirstResponder(pageOverview.collectionView)
        }
        else if activeMode == .pages { activeMode = nil; annotationPalette.isHidden = true; paletteHeightConstraint?.constant = 0 }
        if !visible, readerPresentationNeedsRefresh { refreshDocument(preservePage: pageOverview.selectedPages.first) }
    }
    @objc private func toggleSidebar() { _ = readerCommands.run(.toggleSidebar) }
    func toggleSidebarForTesting() { toggleSidebar() }
    @objc private func showLayoutOptions(_ sender: NSButton) {
        if layoutPopover.isShown { layoutPopover.performClose(sender); return }
        if isOverviewVisible { setOverviewVisible(false) }
        layoutPopover.behavior = .transient
        layoutPopover.contentViewController = ReaderLayoutOptionsController(commands: readerCommands)
        layoutPopover.contentSize = NSSize(width: 280, height: 230)
        layoutPopover.show(relativeTo: sender.bounds, of: sender, preferredEdge: .minY)
    }
    @objc private func toggleContinuous() { readerCommands.setContinuous(!readerCommands.continuous) }
    @objc private func selectMode(_ sender: ModeButton) { if let mode = sender.mode { enterMode(mode) } }
    func enterMode(_ mode: WorkbenchMode) {
        guard pdfView.finishInlineTextEditing() else { return }
        pdfView.contentEditingEnabled = false; contentToolID = nil
        resetDocumentInteractionState(); eraserEnabled = false
        setOverviewVisible(mode == .pages)
        activeMode = mode
        primaryModeButtons.forEach { $0.state = $0.mode == mode ? .on : .off; $0.needsDisplay = true }
        secondaryTools.arrangedSubviews.forEach { secondaryTools.removeArrangedSubview($0); $0.removeFromSuperview() }
        for tool in ReferenceToolStrip.tools(for: mode) {
            let button = ToolStripButton(tool: tool, target: self, action: #selector(runSecondaryTool(_:)))
            if tool.id == "paste-page" { button.isEnabled = clipboard.pages != nil; button.toolTip = "粘贴复制的页面" }
            secondaryTools.addArrangedSubview(button)
        }
        annotationPalette.isHidden = false; paletteHeightConstraint?.constant = 34
        setInteractionStatus("\(mode.statusTitle)模式 · 选择下方工具后才会修改文档")
    }
    @objc private func openOCR() { presentLocalTool("ocr") }
    @objc private func runSecondaryTool(_ button: ToolStripButton) {
        eraserEnabled = false
        if button.toolID.hasPrefix("content-") { selectContentTool(button.toolID); return }
        if activeMode == .pages {
            switch button.toolID {
            case "attach": insertPDF()
            case "extract": extractPages()
            case "delete-page":
                guard !pageOverview.selectedPages.isEmpty else { return }
                let alert = NSAlert(); alert.messageText = "删除选中的 \(pageOverview.selectedPages.count) 页？"
                alert.informativeText = "此操作可通过撤销恢复。"; alert.addButton(withTitle: "删除"); alert.addButton(withTitle: "取消")
                if alert.runModal() == .alertFirstButtonReturn { perform { try performOverviewAction(button.toolID) } }
            default: perform { try performOverviewAction(button.toolID) }
            }
            return
        }
        switch button.toolID {
        case "highlight": highlight()
        case "underline": underline()
        case "strikeout": strikeout()
        case "pen": toggleInkFromPalette()
        case "eraser":
            selectEraser()
        case "text": addFreeText()
        case "shapes": beginShapeTool(pdfView.shapeTool ?? .rectangle)
        case "note": addPopupNote()
        case "signature": addSignature()
        case "selection": resetDocumentInteractionState(); eraserEnabled = false; setInteractionStatus("选择批注 · 点击页面中的批注后可编辑或删除")
        case "insert": insertBlank()
        case "attach": insertPDF()
        case "rotate": rotatePage()
        case "copy-page": perform { try performOverviewAction("copy-page") }
        case "extract": extractPages()
        case "delete-page": deleteCurrentPage()
        case "save": saveDocument()
        case "password": savePasswordProtectedCopy()
        case "export-text": exportPlainText()
        default: setInteractionStatus("暂不支持此工具，文档未修改")
        }
    }

    /// Page overview commands always use the grid selection, never the hidden reader's current page.
    func performOverviewAction(_ id: String) throws {
        let pages = pageOverview.selectedPages
        guard isOverviewVisible, let document = workspace.document, !pages.isEmpty,
              pages.isSubset(of: IndexSet(integersIn: 0..<document.pageCount)) else { throw PDFWorkspaceError.invalidSelection }
        var resultingSelection = pages
        switch id {
        case "rotate": try workspace.rotate(pages: pages, degrees: 90)
        case "copy-page":
            let data = try workspace.copyPages(pages)
            guard clipboard.write(data, type: DocumentClipboard.pagesType) else { throw PDFWorkspaceError.writeFailed }
            secondaryTools.arrangedSubviews.compactMap { $0 as? ToolStripButton }.filter { $0.toolID == "paste-page" }.forEach { $0.isEnabled = true }
            setInteractionStatus("已复制 \(pages.count) 页，可在 PDF 标签间粘贴")
            return
        case "paste-page":
            guard let data = clipboard.pages, let incoming = PDFDocument(data: data) else { throw PDFWorkspaceError.invalidSelection }
            let index = pages.last! + 1
            try workspace.pastePages(data, at: index)
            resultingSelection = IndexSet(integersIn: index..<(index + incoming.pageCount))
        case "insert":
            let index = pages.last! + 1
            try workspace.insertBlank(at: index); resultingSelection = IndexSet(integer: index)
        case "delete-page":
            try workspace.delete(pages: pages)
            resultingSelection = IndexSet(integer: min(pages.first!, workspace.document!.pageCount - 1))
        default: throw PDFWorkspaceError.invalidSelection
        }
        refreshDocument(); pageOverview.restoreSelection(resultingSelection)
    }

    private func clipboardPages(overview: Bool) -> IndexSet {
        if overview { return pageOverview.selectedPages }
        guard let document = pdfView.document else { return [] }
        return IndexSet((readerSidebar.thumbnailView.selectedPages ?? []).compactMap {
            let index = document.index(for: $0); return index == NSNotFound ? nil : index
        })
    }

    func canPageClipboard(_ command: ClipboardCommand, overview: Bool) -> Bool {
        guard workspace.document != nil, overview ? isOverviewVisible : (!isOverviewVisible && readerSidebar.mode == .thumbnails) else { return false }
        let pages = clipboardPages(overview: overview)
        switch command {
        case .selectAll: return true
        case .copy: return !pages.isEmpty && workspace.allowsCopying && workspace.allowsDocumentAssembly
        case .cut: return !pages.isEmpty && pages.count < (workspace.document?.pageCount ?? 0) && workspace.allowsCopying && workspace.allowsDocumentAssembly
        case .paste: return workspace.allowsDocumentAssembly && clipboard.hasPages
        }
    }

    func runPageClipboard(_ command: ClipboardCommand, overview: Bool) {
        guard canPageClipboard(command, overview: overview), let document = workspace.document else { return }
        let pages = clipboardPages(overview: overview)
        perform {
            if command == .selectAll {
                let all = IndexSet(integersIn: 0..<document.pageCount)
                if overview { pageOverview.restoreSelection(all) }
                else { readerSidebar.thumbnailView.selectPages(all) }
                return
            }
            if command == .copy || command == .cut {
                let data = try workspace.copyPages(pages)
                guard clipboard.write(data, type: DocumentClipboard.pagesType) else { throw PDFWorkspaceError.writeFailed }
                if command == .copy { setInteractionStatus("已复制 \(pages.count) 页"); return }
                try deleteSelectedPages(pages)
            } else {
                guard let data = clipboard.pages, let incoming = PDFDocument(data: data) else { return }
                let insertion = (pages.last ?? currentPageIndex) + 1
                try workspace.pastePages(data, at: insertion)
                refreshDocument(preservePage: insertion)
                let added = IndexSet(integersIn: insertion..<(insertion + incoming.pageCount))
                if overview { pageOverview.restoreSelection(added) }
                else { readerSidebar.thumbnailView.selectPages(added) }
                setInteractionStatus("已粘贴 \(incoming.pageCount) 页，可撤销")
            }
            view.window?.makeFirstResponder(overview ? pageOverview.collectionView : readerSidebar.thumbnailView)
        }
    }

    func canAnnotationClipboard(_ command: ClipboardCommand) -> Bool {
        guard !isOverviewVisible, workspace.document != nil else { return false }
        switch command {
        case .copy: return annotationSelection.current(in: workspace) != nil && workspace.allowsCopying
        case .cut: return annotationSelection.current(in: workspace) != nil && workspace.allowsCopying && workspace.allowsCommenting
        case .paste: return workspace.allowsCommenting && (clipboard.hasAnnotation || clipboardText != nil)
        case .selectAll: return false
        }
    }

    @discardableResult func runAnnotationClipboard(_ command: ClipboardCommand) -> Bool {
        // A selected annotation takes precedence over any stale PDF text selection.
        let selection = annotationSelection.current(in: workspace)
        guard command == .paste ? (clipboard.annotation != nil || clipboardText != nil) : selection != nil else { return false }
        guard canAnnotationClipboard(command) else { return true }
        perform {
            if command == .copy || command == .cut, let selection {
                let data = try workspace.copyAnnotation(page: selection.page, index: selection.index)
                guard clipboard.write(data, type: DocumentClipboard.annotationType) else { throw PDFWorkspaceError.writeFailed }
                if command == .cut {
                    try workspace.removeAnnotation(page: selection.page, index: selection.index)
                    try synchronizeAnnotationPresentation(pages: IndexSet(integer: selection.page))
                    annotationSelection.clear(); selectedShape = nil; selectedInk = nil; pdfView.clearShapeSelection()
                }
                setInteractionStatus(command == .cut ? "已剪切批注，可撤销" : "已复制批注")
            } else if command == .paste, let data = clipboard.annotation {
                let page = currentPageIndex
                var index: Int?
                index = try workspace.pasteAnnotation(data, page: page)
                try synchronizeAnnotationPresentation(pages: IndexSet(integer: page))
                if let index { selectAnnotation(page: page, index: index) }
                setInteractionStatus("已粘贴批注，可撤销")
            } else if command == .paste, let text = clipboardText,
                      let page = workspace.document?.page(at: currentPageIndex) {
                let crop = page.bounds(for: .cropBox), pageIndex = currentPageIndex
                let bounds = CGRect(x: crop.minX + min(24, crop.width / 10), y: crop.midY,
                                    width: min(260, crop.width * 0.8), height: min(100, crop.height * 0.4))
                try workspace.addText(text, page: pageIndex, bounds: bounds)
                try synchronizeAnnotationPresentation(pages: IndexSet(integer: pageIndex))
                setInteractionStatus("已粘贴为可编辑文字批注，可撤销")
            }
        }
        return true
    }
    private var clipboardText: String? {
        guard let text = clipboard.pasteboard.string(forType: .string), !text.isEmpty, text.utf8.count <= 100_000 else { return nil }
        return text
    }

    /// Deletes the active page selection as one undoable operation.
    func deleteSelectedPages(_ pages: IndexSet) throws {
        guard let first = pages.first else { return }
        let scale = pdfView.scaleFactor
        try workspace.delete(pages: pages)
        let target = min(first, max(0, (workspace.document?.pageCount ?? 1) - 1))
        refreshDocument(preservePage: target, scale: scale)
        if isOverviewVisible {
            pageOverview.restoreSelection(IndexSet(integer: target))
            view.window?.makeFirstResponder(pageOverview.collectionView)
        } else {
            view.window?.makeFirstResponder(readerSidebar.thumbnailView)
        }
        setInteractionStatus("已删除 \(pages.count) 页，可撤销恢复")
    }
    private func showShapesMenu(_ source: NSView) {
        let menu = NSMenu()
        for (title, action) in [("矩形", #selector(addRectangle)), ("椭圆", #selector(addEllipse)), ("直线", #selector(addLineAnnotation)), ("箭头", #selector(addArrow)), ("双向箭头", #selector(addDoubleArrow))] {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: ""); item.target = self; menu.addItem(item)
        }
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: source.bounds.height), in: source)
    }
    @objc private func toggleInkFromPalette() { let item = NSMenuItem(); toggleInk(item) }
    @objc private func showToolPalette() { enterMode(.annotation) }
    @objc private func noop() {}
    @objc private func changeMode(_ sender: NSPopUpButton) { pdfView.displayMode = [.singlePageContinuous, .singlePage, .twoUpContinuous][sender.indexOfSelectedItem] }
    @objc func undo() { restoreHistory { workspace.undo() } }
    @objc func redo() { restoreHistory { workspace.redo() } }
    private func restoreHistory(_ operation: () -> PDFHistoryPresentationChange?) {
        let page = currentPageIndex, scale = pdfView.scaleFactor
        guard let change = operation() else { return }
        perform {
            switch change {
            case .annotations(let pages): try self.synchronizeAnnotationPresentation(pages: pages)
            case .document: self.refreshDocument(preservePage: page, scale: scale)
            }
        }
    }
    func perform(_ operation: () throws -> Void) { do { try operation() } catch let error as PDFWorkspaceError { showError(WorkbenchMessage.error(error)) } catch { showError("操作失败：\(error.localizedDescription)") } }
    private func showError(_ text: String) { let alert = NSAlert(); alert.alertStyle = .warning; alert.messageText = "无法完成操作"; alert.informativeText = text; alert.runModal() }

    // Menu actions keep the toolbar readable while exposing the full foundational slice.
    @objc func rotatePage() { mutate { try workspace.rotate(page: currentPageIndex, degrees: 90) } }
    @objc func rotatePageLeft() { mutate { try workspace.rotate(page: currentPageIndex, degrees: -90) } }
    @objc func selectEraser() {
        guard pdfView.finishInlineTextEditing() else { return }
        cancelPendingPlacement(); pdfView.clearShapeSelection(); pdfView.annotationManipulationEnabled = false
        pdfView.inkEnabled = false; pdfView.placementEnabled = false; eraserEnabled = true
        setInteractionStatus("橡皮：点击删除整条手绘笔迹，可撤销；暂不支持局部擦除")
    }
    @objc func insertBlank() { mutate { try workspace.insertBlank(at: currentPageIndex + 1) } }
    @objc func insertPDF() {
        let p = NSOpenPanel()
        p.title = "附加文件并合并为 PDF"
        p.message = "支持 PDF、图片、TXT/MD/RTF 和纯文字 DOCX；RTF/DOCX 重新分页，不保留 Word 完整版式。其他格式请先导出 PDF。"
        p.allowedContentTypes = (["pdf"] + FilePDFImport.imageExtensions + FilePDFImport.textExtensions).compactMap { UTType(filenameExtension: $0) }
        p.allowsMultipleSelection = true; p.canChooseDirectories = false; p.resolvesAliases = false
        guard p.runModal() == .OK else { return }
        let index = isOverviewVisible ? (pageOverview.selectedPages.last.map { $0 + 1 } ?? 0) : currentPageIndex + 1
        queueFileMerge(p.urls, at: index)
    }
    @objc func movePageUp() { guard let doc = workspace.document, let text = textPrompt("移动当前页", placeholder: "目标页码 1-\(doc.pageCount)") else { return }; guard let destination = PageJump.index(for: text, pageCount: doc.pageCount) else { showError(WorkbenchMessage.error(.invalidPage)); return }; let from = currentPageIndex; mutate { try workspace.movePage(from: from, to: destination) } }
    @objc func extractPages() {
        guard let document = workspace.document else { return }
        let pages: IndexSet
        if isOverviewVisible {
            pages = pageOverview.selectedPages
            guard !pages.isEmpty else { return }
        } else {
            guard let rangeText = textPrompt("提取页面", placeholder: "例如 1-3,5") else { return }
            do { pages = try PageRange.parse(rangeText, count: document.pageCount) }
            catch { showError(WorkbenchMessage.error(.invalidPageRange)); return }
        }
        let panel = NSSavePanel(); panel.allowedContentTypes = [.pdf]; panel.nameFieldStringValue = "提取页面.pdf"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        _ = performExtractPages(pages, to: url) { self.confirmEncryptedExtraction() }
    }

    @discardableResult func performExtractPages(_ pages: IndexSet, to url: URL, confirmProtectionRemoval: () -> Bool) -> Bool {
        if workspace.isEncryptedSource, !confirmProtectionRemoval() { return false }
        do { try workspace.extract(pages: pages, to: url); return true }
        catch let error as PDFWorkspaceError { showError(WorkbenchMessage.error(error)); return false }
        catch { showError("操作失败：\(error.localizedDescription)"); return false }
    }

    private func confirmEncryptedExtraction() -> Bool {
        let alert = NSAlert(); alert.alertStyle = .warning; alert.messageText = "导出将移除原 PDF 保护"; alert.informativeText = ExtractPagesProtection.warning
        alert.addButton(withTitle: "导出明文副本"); alert.addButton(withTitle: "取消")
        return alert.runModal() == .alertFirstButtonReturn
    }
    @objc func deleteCurrentPage() { guard let doc = workspace.document, let text = textPrompt("选择要删除的页面", placeholder: "例如 \(currentPageIndex + 1) 或 2-4") else { return }; let pages: IndexSet; do { pages = try PageRange.parse(text, count: doc.pageCount) } catch { showError(WorkbenchMessage.error(.invalidPageRange)); return }; let alert = NSAlert(); alert.messageText = "删除选中的 \(pages.count) 页？"; alert.informativeText = "此操作可通过“撤销”恢复。"; alert.addButton(withTitle: "删除"); alert.addButton(withTitle: "取消"); guard alert.runModal() == .alertFirstButtonReturn else { return }; mutate { try workspace.delete(pages: pages) } }
    @objc func addFreeText() {
        guard workspace.document != nil, workspace.allowsCommenting, pdfView.finishInlineTextEditing() else { return }
        beginPlacement(.freeText(""), accessibilityName: "页面内文字输入模式")
        pdfView.annotationManipulationEnabled = true
        pdfView.freeTextInputEnabled = true
        setInteractionStatus("空白处直接输入 · 已有文字单击选中拖动、双击修改 · Esc 退出")
    }
    @objc func highlight() { addMarkup(.highlight, color: NSColor.systemYellow.withAlphaComponent(0.45)) }; @objc func underline() { addMarkup(.underline, color: .systemIndigo) }; @objc func strikeout() { addMarkup(.strikeOut, color: .systemRed) }
    private func addMarkup(_ kind: PDFAnnotationSubtype, color: NSColor) {
        guard workspace.document != nil, workspace.allowsCommenting else { return }
        let selection = pdfView.currentSelection
        if activeMarkup?.kind == kind, selection == nil {
            resetDocumentInteractionState(); return
        }
        resetDocumentInteractionState(); eraserEnabled = false
        activeMarkup = (kind, color); pdfView.markupSelectionEnabled = true
        let id = kind == .highlight ? "highlight" : (kind == .underline ? "underline" : "strikeout")
        let title = kind == .highlight ? "高亮" : (kind == .underline ? "下划线" : "删除线")
        secondaryTools.arrangedSubviews.compactMap { $0 as? ToolStripButton }.forEach { $0.state = $0.toolID == id ? .on : .off }
        pdfView.setAccessibilityLabel("PDF 文档视图，\(title)模式，拖选文字后应用批注；Esc 退出")
        setInteractionStatus("\(title) · 拖选文字后松开鼠标应用 · 可连续标记 · Esc 退出")
        if let selection, selection.string?.contains(where: { !$0.isWhitespace }) == true {
            applyMarkup(selection, kind: kind, color: color)
        }
    }
    private func applyMarkup(_ selection: PDFSelection, kind: PDFAnnotationSubtype, color: NSColor) {
        guard workspace.allowsCommenting, let presentation = pdfView.document,
              let workingDocument = workspace.document else { return }
        let mapped = PDFSelection(document: workingDocument)
        for fragment in selection.selectionsByLine() {
            guard let sourcePage = fragment.pages.first else { continue }
            let pageIndex = presentation.index(for: sourcePage)
            guard pageIndex != NSNotFound, let targetPage = workingDocument.page(at: pageIndex),
                  let targetSelection = targetPage.selection(for: fragment.bounds(for: sourcePage)) else { continue }
            mapped.add(targetSelection)
        }
        let pages = IndexSet(mapped.pages.map { workingDocument.index(for: $0) }.filter { $0 != NSNotFound })
        guard !pages.isEmpty else { return }
        mutateAnnotations(pages: pages) { try workspace.addMarkup(selection: mapped, kind: kind, color: color) }
        pdfView.setCurrentSelection(nil, animate: false)
    }
    private func clearMarkupTool() {
        activeMarkup = nil; pdfView.markupSelectionEnabled = false
        for button in secondaryTools.arrangedSubviews.compactMap({ $0 as? ToolStripButton }) where ["highlight", "underline", "strikeout"].contains(button.toolID) { button.state = .off }
    }
    @objc func toggleInk(_ sender: NSMenuItem) {
        cancelPendingPlacement(); pdfView.clearShapeSelection(); pdfView.annotationManipulationEnabled = false
        pdfView.contentEditingEnabled = false; contentToolID = nil; eraserEnabled = false
        pendingPlacement = nil
        pdfView.placementEnabled = false
        pdfView.inkEnabled.toggle()
        inkMenuItem = sender
        syncInkMenuState()
        pdfView.setAccessibilityLabel(pdfView.inkEnabled ? "PDF 文档视图，手绘模式" : "PDF 文档视图")
        if pdfView.inkEnabled {
            pdfView.inkStyle = drawingInkStyle
            inkStyleBar.configure(tool: .line, style: drawingInkStyle)
            inkStyleBar.isHidden = false; shapeStyleBar.isHidden = true
            annotationPalette.isHidden = false; paletteHeightConstraint?.constant = 84
            secondaryTools.arrangedSubviews.compactMap { $0 as? ToolStripButton }.forEach { $0.state = $0.toolID == "pen" ? .on : .off }
            setInteractionStatus("手绘模式 · 拖动时实时显示笔迹 · 属性栏调整颜色/线宽 · Esc 退出")
        } else {
            inkStyleBar.isHidden = true; if activeMode != nil { paletteHeightConstraint?.constant = 34 }
            refreshPersistentDocumentStatus()
        }
    }
    func changeInkStyle(_ style: AnnotationStyle) {
        guard style.lineWidth.isFinite, AnnotationStyle.supportedLineWidths.contains(style.lineWidth) else { return }
        if selectedInk != nil {
            guard let selected = currentSelectedInk else { clearSelectedInk(); return }
            pdfView.clearStylePreview()
            mutateAnnotations(pages: IndexSet(integer: selected.page)) { try workspace.updateInkStyle(page:selected.page,index:selected.index,style:AnnotationStyle(strokeColor:style.strokeColor,fillColor:nil,lineWidth:style.lineWidth)) }
            selectAnnotation(page:selected.page,index:selected.index)
            return
        }
        drawingInkStyle = AnnotationStyle(strokeColor: style.strokeColor, fillColor: nil, lineWidth: style.lineWidth)
        pdfView.inkStyle = drawingInkStyle
    }
    func previewInkStyle(_ style: AnnotationStyle) {
        if selectedInk != nil {
            guard currentSelectedInk != nil else { clearSelectedInk(); return }
            pdfView.previewSelectedInkStyle(style)
        } else { changeInkStyle(style) }
    }
    private var currentSelectedInk: (page:Int,index:Int)? {
        guard let s = selectedInk, let document = workspace.document,
              s.document == ObjectIdentifier(document), s.revision == workspace.revision,
              let tracked = annotationSelection.current(in:workspace), tracked.page == s.page, tracked.index == s.index,
              document.page(at:s.page)?.annotations[safe:s.index]?.type == "Ink" else { return nil }
        return (s.page,s.index)
    }
    private func clearSelectedInk() {
        selectedInk = nil; pdfView.clearShapeSelection()
        inkStyleBar.configure(tool:.line,style:drawingInkStyle)
        inkStyleBar.isHidden = true
    }
    @objc func addSignature() {
        guard workspace.document != nil, workspace.allowsCommenting else { return }
        let sheet = SignaturePanel()
        guard let strokes = sheet.run(window: view.window), !strokes.isEmpty else { return }
        beginPlacement(.signature(strokes), accessibilityName: "签名放置模式")
    }
    @objc func printDocument() {
        if let reason = DocumentOperationMessage.printBlockReason(
            hasDocument: workspace.document != nil,
            isEncryptedSource: workspace.isEncryptedSource,
            allowsPrinting: workspace.allowsPrinting
        ) { showError(reason); return }
        guard let document = workspace.document else { return }
        let info = NSPrintInfo.shared.copy() as! NSPrintInfo
        guard let operation = document.printOperation(for: info, scalingMode: .pageScaleToFit, autoRotate: true), let window = view.window else {
            showError("无法创建打印任务。"); return
        }
        operation.runModal(for: window, delegate: nil, didRun: nil, contextInfo: nil)
    }

    @objc func saveFlattenedCopy() {
        let warning = NSAlert()
        warning.messageText = "存储打平副本"
        warning.informativeText = "打平会将可打印批注烧录到页面外观中。这不是安全涂黑；弹出式或不可打印批注可能不会显示。输出不会保留原 PDF 的加密。"
        warning.addButton(withTitle: "继续")
        warning.addButton(withTitle: "取消")
        guard warning.runModal() == .alertFirstButtonReturn else { return }
        let panel = NSSavePanel(); panel.allowedContentTypes = [.pdf]; panel.nameFieldStringValue = "打平副本.pdf"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        perform { try workspace.saveFlattenedCopy(to: url) }
    }

    @objc func savePasswordProtectedCopy() {
        guard PasswordProtectedCopyPolicy.isAvailable(in: workspace) else {
            showError("已有加密 PDF 可在本次工作区中查看和编辑，但当前版本无法可靠验证并保留其原始底层权限，因此拒绝重新加密导出。新建或未加密来源仍可创建密码保护副本。")
            return
        }
        let panel = NSSavePanel(); panel.allowedContentTypes = [.pdf]; panel.nameFieldStringValue = "密码保护副本.pdf"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        guard let credentials = DocumentToolPanels.encryptionCredentials() else { return }
        perform {
            try workspace.saveEncryptedCopy(
                to: url,
                userPassword: credentials.userPassword,
                ownerPassword: credentials.ownerPassword
            )
        }
    }


    @objc func exportPlainText() {
        exportText(title: "导出纯文本", suggestedName: "文档.txt", contentType: .plainText) {
            try workspace.exportPlainText()
        }
    }

    @objc func exportAnnotationSummaryText() {
        exportAnnotationSummary(.plainText, suggestedName: "批注摘要.txt", contentType: .plainText)
    }

    @objc func exportAnnotationSummaryMarkdown() {
        exportAnnotationSummary(.markdown, suggestedName: "批注摘要.md", contentType: UTType(filenameExtension: "md") ?? .plainText)
    }

    @objc func exportAnnotationSummaryHTML() {
        exportAnnotationSummary(.html, suggestedName: "批注摘要.html", contentType: .html)
    }

    private func exportAnnotationSummary(_ format: AnnotationSummaryFormat, suggestedName: String, contentType: UTType) {
        exportText(title: "导出批注摘要", suggestedName: suggestedName, contentType: contentType) {
            try workspace.exportAnnotationSummary(format: format)
        }
    }

    private func exportText(title: String, suggestedName: String, contentType: UTType, makeText: () throws -> String) {
        let panel = NSSavePanel(); panel.title = title; panel.allowedContentTypes = [contentType]; panel.nameFieldStringValue = suggestedName
        guard panel.runModal() == .OK, let url = panel.url else { return }
        perform {
            let text = try makeText()
            try SafeTextExport.write(text, to: url, protection: sourceProtection)
        }
    }

    @objc func addPopupNote() {
        guard let configuration = DocumentToolPanels.noteConfiguration() else { return }
        beginPlacement(.note(configuration), accessibilityName: "便筺放置模式")
    }

    @objc func addRectangle() { beginShape(.rectangle) }
    @objc func addEllipse() { beginShape(.ellipse) }
    @objc func addLineAnnotation() { beginLine(.none) }
    @objc func addArrow() { beginLine(.arrow) }
    @objc func addDoubleArrow() { beginLine(.doubleArrow) }

    private func beginShape(_ shape: AnnotationShape) {
        beginShapeTool(shape == .rectangle ? .rectangle : .ellipse)
    }

    private func beginLine(_ endings: LineEndings) {
        beginShapeTool(endings == .none ? .line : (endings == .arrow ? .arrow : .doubleArrow))
    }

    func beginShapeTool(_ tool: ShapeToolKind) {
        guard workspace.document != nil, workspace.allowsCommenting else { return }
        selectedShape = nil; pdfView.clearShapeSelection(); eraserEnabled = false
        pdfView.annotationManipulationEnabled = true
        pdfView.contentEditingEnabled = false; contentToolID = nil
        let configuration = AnnotationConfiguration(contents: nil, style: drawingShapeStyle)
        switch tool {
        case .rectangle: beginPlacement(.shape(.rectangle, configuration), accessibilityName: "矩形实时预览")
        case .ellipse: beginPlacement(.shape(.ellipse, configuration), accessibilityName: "椭圆实时预览")
        case .line: beginPlacement(.line(.none, configuration), accessibilityName: "直线实时预览")
        case .arrow: beginPlacement(.line(.arrow, configuration), accessibilityName: "箭头实时预览")
        case .doubleArrow: beginPlacement(.line(.doubleArrow, configuration), accessibilityName: "双向箭头实时预览")
        }
        pdfView.shapeTool = tool; pdfView.shapeStyle = drawingShapeStyle
        shapeStyleBar.configure(tool: tool, style: drawingShapeStyle)
        shapeStyleBar.isHidden = false; annotationPalette.isHidden = false; paletteHeightConstraint?.constant = 84
        secondaryTools.arrangedSubviews.compactMap { $0 as? ToolStripButton }.forEach { $0.state = $0.toolID == "shapes" ? .on : .off }
        setInteractionStatus("拖动绘制形状 · 可调整颜色、线宽和透明度 · 选中形状可移动/缩放 · Esc 退出")
    }

    func changeShapeStyle(_ style: AnnotationStyle) {
        pdfView.clearStylePreview()
        drawingShapeStyle = style; pdfView.shapeStyle = style
        if let tool = pdfView.shapeTool { updateShapePlacement(tool, style: style) }
        if let s = selectedShape, s.revision == workspace.revision {
            guard let bounds = workspace.document?.page(at: s.page)?.annotations[safe: s.index]?.bounds else { return }
            mutateAnnotations(pages: IndexSet(integer: s.page)) { try workspace.updateShape(page: s.page, index: s.index, bounds: bounds, style: style) }
            selectAnnotation(page: s.page, index: s.index)
        } else if let tool = pdfView.shapeTool {
            // Refresh placement's immutable style without a modal or document mutation.
            beginShapeTool(tool)
        }
    }

    func previewShapeStyle(_ style: AnnotationStyle) { pdfView.previewSelectedStyle(style) }

    private func updateShapePlacement(_ tool: ShapeToolKind, style: AnnotationStyle) {
        let c = AnnotationConfiguration(contents: nil, style: style)
        switch tool {
        case .rectangle: pendingPlacement = .shape(.rectangle, c)
        case .ellipse: pendingPlacement = .shape(.ellipse, c)
        case .line: pendingPlacement = .line(.none, c)
        case .arrow: pendingPlacement = .line(.arrow, c)
        case .doubleArrow: pendingPlacement = .line(.doubleArrow, c)
        }
    }

    func beginPlacement(_ placement: PendingPlacement, accessibilityName: String) {
        guard workspace.document != nil, workspace.allowsCommenting else { return }
        guard pdfView.finishInlineTextEditing() else { return }
        pdfView.freeTextInputEnabled = false
        clearMarkupTool()
        selectedInk = nil
        inkStyleBar.isHidden = true
        pdfView.shapeTool = nil; shapeStyleBar.isHidden = true; selectedShape = nil; pdfView.clearShapeSelection()
        if activeMode != nil { paletteHeightConstraint?.constant = 34 }
        pendingPlacement = placement
        pdfView.contentEditingEnabled = false; contentToolID = nil; eraserEnabled = false
        pdfView.inkEnabled = false
        pdfView.placementEnabled = true
        syncInkMenuState()
        pdfView.setAccessibilityLabel("“\(accessibilityName)”，点击 PDF 页面放置，或拖动调整大小；Esc 取消")
        setInteractionStatus("点击页面放置 · 拖动可指定大小 · Esc 取消且不修改文档")
    }

    private func cancelPendingPlacement() {
        _ = pdfView.finishInlineTextEditing(commit: false)
        pdfView.freeTextInputEnabled = false
        clearMarkupTool()
        inkStyleBar.widthSlider.cancelShapeTracking()
        selectedInk = nil
        inkStyleBar.isHidden = true
        pendingPlacement = nil
        pdfView.placementEnabled = false
        pdfView.shapeTool = nil; shapeStyleBar.isHidden = true; selectedShape = nil
        if activeMode != nil { paletteHeightConstraint?.constant = 34 }
        pdfView.setAccessibilityLabel("PDF 文档视图")
        syncInkMenuState()
        refreshPersistentDocumentStatus()
    }

    func resetDocumentInteractionState() {
        guard pdfView.finishInlineTextEditing() else { return }
        pdfView.freeTextInputEnabled = false
        clearMarkupTool()
        inkStyleBar.widthSlider.cancelShapeTracking()
        selectedInk = nil
        inkStyleBar.isHidden = true
        pdfView.annotationManipulationEnabled = true
        selectedShape = nil; pdfView.clearShapeSelection(); pdfView.shapeTool = nil; shapeStyleBar.isHidden = true
        pdfView.contentEditingEnabled = false; contentToolID = nil
        pendingPlacement = nil
        annotationSelection.clear()
        pdfView.inkEnabled = false
        pdfView.placementEnabled = false
        syncInkMenuState()
        pdfView.setCurrentSelection(nil, animate: false)
        pdfView.setAccessibilityLabel("PDF 文档视图")
        refreshPersistentDocumentStatus()
    }

    func captureAdmittedSource(_ url: URL) { sourceProtection = SourceProtection.capture(url) }
    func registerInkMenuItem(_ menuItem: NSMenuItem?) {
        inkMenuItem = menuItem
        syncInkMenuState()
    }

    func setInteractionStatus(_ text: String, toolTip: String? = nil) {
        status.stringValue = text
        status.toolTip = toolTip
    }

    private func syncInkMenuState() {
        inkMenuItem?.state = pdfView.inkEnabled ? .on : .off
    }

    private func refreshPersistentDocumentStatus() {
        let hasDocument = workspace.document != nil
        let encrypted = workspace.isEncryptedSource
        let message = DocumentStatusPresentation.message(hasDocument: hasDocument, isEncryptedSource: encrypted)
        let toolTip = DocumentStatusPresentation.toolTip(hasDocument: hasDocument, isEncryptedSource: encrypted)
        setInteractionStatus(message, toolTip: toolTip)
        pdfView.toolTip = toolTip
    }

    private func applyPlacement(from start: CGPoint, to end: CGPoint, page pageIndex: Int) {
        guard let placement = pendingPlacement,
              let page = workspace.document?.page(at: pageIndex) else { return }
        if pdfView.freeTextInputEnabled {
            beginFreeTextInput(from: start, to: end, page: pageIndex); return
        }
        let persistentTool = pdfView.shapeTool
        if persistentTool == nil { pendingPlacement = nil; pdfView.placementEnabled = false }
        pdfView.setAccessibilityLabel("PDF 文档视图")
        refreshPersistentDocumentStatus()
        do {
            switch placement {
            case .note(let configuration):
                let bounds = try placement.bounds(from: start, to: end, within: page.bounds(for: .cropBox))
                mutateAnnotations(pages: IndexSet(integer: pageIndex)) { try workspace.addPopupNote(configuration.contents, author: configuration.author, page: pageIndex, bounds: bounds, color: configuration.color) }
            case .freeText(let text):
                let bounds = try placement.bounds(from: start, to: end, within: page.bounds(for: .cropBox))
                mutateAnnotations(pages: IndexSet(integer: pageIndex)) { try workspace.addText(text, page: pageIndex, bounds: bounds) }
            case .signature(let strokes):
                // Ink bounds include a full line-width margin; keep that margin
                // inside the crop even when the pointer is on the page edge.
                let crop = page.bounds(for: .cropBox).insetBy(dx: 2.2, dy: 2.2)
                let a = CGPoint(x: min(max(start.x, crop.minX), crop.maxX), y: min(max(start.y, crop.minY), crop.maxY))
                let z = CGPoint(x: min(max(end.x, crop.minX), crop.maxX), y: min(max(end.y, crop.minY), crop.maxY))
                let bounds = try placement.bounds(from: a, to: z, within: crop, rotation: page.rotation)
                let mapped = PendingPlacement.signatureStrokes(strokes, in: bounds, rotation: page.rotation)
                mutateAnnotations(pages: IndexSet(integer: pageIndex)) { try workspace.addInk(strokes: mapped, page: pageIndex, color: .black, width: 2.2) }
            case .shape(let shape, let configuration):
                let bounds = try ToolInputValidation.normalizedDrag(from: start, to: end, within: page.bounds(for: .cropBox))
                mutateAnnotations(pages: IndexSet(integer: pageIndex)) { try workspace.addShape(shape, page: pageIndex, bounds: bounds, style: configuration.style) }
            case .line(let endings, let configuration):
                guard start != end else { throw PDFWorkspaceError.invalidSelection }
                mutateAnnotations(pages: IndexSet(integer: pageIndex)) { try workspace.addLine(from: start, to: end, page: pageIndex, style: configuration.style, endings: endings) }
            }
            if persistentTool != nil, let index = workspace.document?.page(at: pageIndex)?.annotations.indices.last {
                selectAnnotation(page: pageIndex, index: index)
            }
        } catch let error as PDFWorkspaceError { showError(WorkbenchMessage.error(error)) }
        catch { showError("操作失败：\(error.localizedDescription)") }
    }

    private func selectAnnotation(page: Int, index: Int) {
        guard let annotation = workspace.document?.page(at: page)?.annotations[safe: index],
              annotation.type != "Widget", annotation.type != "Link" else {
            annotationSelection.clear()
            return
        }
        if eraserEnabled {
            guard annotation.type == "Ink" else { setInteractionStatus("橡皮仅处理手绘笔迹"); return }
            mutateAnnotations(pages: IndexSet(integer: page)) { try workspace.removeAnnotation(page: page, index: index) }; return
        }
        annotationSelection.select(page: page, index: index, in: workspace)
        if let tool = ShapeToolKind.annotationKind(annotation) {
            selectedInk = nil; inkStyleBar.isHidden = true
            selectedShape = (page, index, workspace.revision)
            pdfView.selectShape(page: page, index: index)
            shapeStyleBar.configure(tool: tool, style: AnnotationStyle(annotation: annotation), mode: pdfView.shapeTool == nil ? .selection : .drawing, enabled: workspace.allowsCommenting)
            shapeStyleBar.isHidden = false; annotationPalette.isHidden = false; paletteHeightConstraint?.constant = 84
        } else if annotation.type == "Ink", let document = workspace.document {
            selectedShape = nil; pendingPlacement = nil
            pdfView.placementEnabled = false; pdfView.shapeTool = nil
            selectedInk = (page,index,workspace.revision,ObjectIdentifier(document))
            pdfView.selectInk(page:page,index:index)
            inkStyleBar.configure(tool:.line,style:AnnotationStyle(annotation:annotation),mode:.selection,enabled:workspace.allowsCommenting,selectionTitle:"所选笔迹")
            inkStyleBar.isHidden = false; shapeStyleBar.isHidden = true
            annotationPalette.isHidden = false; paletteHeightConstraint?.constant = 84
        } else { selectedShape = nil; selectedInk = nil; pdfView.clearShapeSelection(); shapeStyleBar.isHidden = true; inkStyleBar.isHidden = true; if activeMode != nil { paletteHeightConstraint?.constant = 34 }
            if annotation.type == "FreeText", workspace.allowsCommenting { pdfView.selectShape(page: page, index: index) }
        }
        status.stringValue = annotation.type == "FreeText" ? "文字已选中 · 拖动移动 · 方向键微调 · 双击修改文字与顶栏样式" : "已选中第 \(page + 1) 页的第 \(index + 1) 个批注（\(annotation.type ?? "Unknown")）"
    }

    @objc func editSelectedAnnotation() {
        guard let selection = annotationSelection.current(in: workspace),
              let page = workspace.document?.page(at: selection.page),
              let annotation = page.annotations[safe: selection.index],
              annotation.type != "Widget", annotation.type != "Link" else {
            showError("请先点击选择可编辑批注；表单和链接不在此工具中编辑。")
            return
        }
        if annotation.type == "FreeText" {
            editFreeTextInline(page: selection.page, annotationIndex: selection.index); return
        }
        if annotation.type == "Ink" {
            selectAnnotation(page:selection.page,index:selection.index)
            setInteractionStatus("所选笔迹 · 属性栏调整颜色和线宽 · 滑块松手后一次应用，可撤销")
            return
        }
        guard let configuration = DocumentToolPanels.editConfiguration(annotation: annotation) else { return }
        guard page.bounds(for: .mediaBox).contains(configuration.bounds) else {
            showError("批注矩形必须完全位于页面 MediaBox 内。")
            return
        }
        mutateAnnotations(pages: IndexSet(integer: selection.page)) {
            try workspace.editAnnotation(
                page: selection.page,
                index: selection.index,
                contents: configuration.contents,
                bounds: configuration.bounds,
                color: configuration.color
            )
        }
    }

    private var canDeleteSelectedAnnotation: Bool {
        workspace.allowsCommenting && annotationSelection.current(in: workspace) != nil
    }
    @objc func deleteSelectedAnnotation() {
        guard canDeleteSelectedAnnotation, let selection = annotationSelection.current(in: workspace) else { return }
        mutateAnnotations(pages: IndexSet(integer: selection.page)) { try workspace.removeAnnotation(page: selection.page, index: selection.index) }
        selectedShape = nil; selectedInk = nil; annotationSelection.clear(); pdfView.clearShapeSelection()
        shapeStyleBar.isHidden = true
        if !pdfView.inkEnabled { inkStyleBar.isHidden = true }
        setInteractionStatus("已删除批注，可通过 ⌘Z 撤销")
    }

    @objc @discardableResult func duplicateCurrentPage() -> Bool {
        guard workspace.document != nil else { return false }
        let page = currentPageIndex
        do {
            try workspace.duplicatePage(at: page, insertingAt: page + 1)
            refreshDocument(preservePage: page)
            return true
        } catch let error as PDFWorkspaceError { showError(WorkbenchMessage.error(error)); return false }
        catch { showError("操作失败：\(error.localizedDescription)"); return false }
    }

    @objc func cropCurrentPage() {
        guard let page = workspace.document?.page(at: currentPageIndex),
              let rectangle = DocumentToolPanels.cropRectangle(mediaBox: page.bounds(for: .mediaBox)) else { return }
        _ = applyCrop(rectangle)
    }

    @discardableResult func applyCrop(_ rectangle: CGRect) -> Bool {
        guard workspace.document != nil else { return false }
        let page = currentPageIndex
        do {
            try workspace.cropPage(at: page, to: rectangle)
            refreshDocument(preservePage: page)
            return true
        } catch let error as PDFWorkspaceError { showError(WorkbenchMessage.error(error)); return false }
        catch { showError("操作失败：\(error.localizedDescription)"); return false }
    }

    @objc func showOutline() {
        _ = readerCommands.run(.outline)
    }

    @objc func addCurrentPageBookmark() {
        guard let label = textPrompt("添加顶层书签", placeholder: "书签名称"), !label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        mutate { try workspace.addBookmark(label: label, page: currentPageIndex) }
    }

    @objc func renameTopLevelBookmark() { editTopLevelBookmark(removing: false) }
    @objc func removeTopLevelBookmark() { editTopLevelBookmark(removing: true) }

    private func editTopLevelBookmark(removing: Bool) {
        guard let document = workspace.document else { return }
        let topLevel = OutlineProjection.make(from: document).filter { $0.topLevelIndex != nil }
        guard !topLevel.isEmpty else { showError("当前文档没有可编辑的顶层书签。"); return }
        let popup = NSPopUpButton(frame: NSRect(x: 0, y: 0, width: 360, height: 26))
        popup.addItems(withTitles: topLevel.map { $0.label + ($0.pageNumber.map { " — 第 \($0) 页" } ?? "") })
        let alert = NSAlert(); alert.messageText = removing ? "删除顶层书签" : "重命名顶层书签"
        alert.informativeText = OutlineEditDisclosure.information(removing: removing)
        alert.accessoryView = popup; alert.addButton(withTitle: "继续"); alert.addButton(withTitle: "取消")
        guard alert.runModal() == .alertFirstButtonReturn, topLevel.indices.contains(popup.indexOfSelectedItem),
              let index = topLevel[popup.indexOfSelectedItem].topLevelIndex else { return }
        if removing {
            mutate { try workspace.removeBookmark(at: index) }
        } else if let label = textPrompt("重命名顶层书签", placeholder: topLevel[popup.indexOfSelectedItem].label) {
            mutate { try workspace.renameBookmark(at: index, label: label) }
        }
    }
    private func mutateAnnotations(pages: IndexSet, _ operation: () throws -> Void) {
        perform {
            try operation()
            try self.synchronizeAnnotationPresentation(pages: pages)
        }
    }
    func synchronizeAnnotationPresentation(pages: IndexSet) throws {
            invalidateCopilotIfChanged()
            guard !isOverviewVisible, !readerPresentationNeedsRefresh else {
                self.refreshDocument(); return
            }
            guard let displayed = self.pdfView.document, let committed = self.workspace.document,
                  displayed.pageCount == committed.pageCount,
                  pages.allSatisfy({ displayed.page(at: $0) != nil && committed.page(at: $0) != nil }) else {
                self.refreshDocument(); return
            }
            let copies = try pages.map { index -> (PDFPage, PDFPage, AnnotationPresentationBacking) in
                // PDFPage.copy() alone shares annotation objects. Moving them
                // would silently remove annotations from the authoritative page.
                // Serialize/reparse only this page to isolate objects AND /AP.
                guard let target = displayed.page(at: index), let bytes = committed.page(at: index)?.dataRepresentation,
                      let source = PDFDocument(data: bytes), let detached = source.page(at: 0) else {
                    throw PDFWorkspaceError.unreadableDocument
                }
                return (target, detached, AnnotationPresentationBacking(bytes: bytes, source: source))
            }
            for (target, detached, backing) in copies {
                for annotation in target.annotations { target.removeAnnotation(annotation) }
                for annotation in detached.annotations {
                    // PDFKit's appearance queue can outlive removal, a later
                    // undo, or the reader itself. Tie /AP backing to the object
                    // the queue retains, not to the controller's current page.
                    objc_setAssociatedObject(annotation, &AnnotationPresentationBacking.associationKey, backing, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
                    detached.removeAnnotation(annotation); target.addAnnotation(annotation)
                }
            }
            if self.selectedInk != nil { self.clearSelectedInk() }
            if self.selectedShape != nil { self.selectedShape = nil; self.pdfView.clearShapeSelection() }
            _ = self.annotationSelection.current(in: self.workspace)
            self.pdfView.refreshCommittedInkRendering()
            self.readerSession.didUpdateAnnotations(snapshot: nil)
            self.pdfView.needsDisplay = true
            self.undoButton?.isEnabled = self.workspace.canUndo; self.redoButton?.isEnabled = self.workspace.canRedo
            self.refreshPersistentDocumentStatus()
    }
    private func mutate(_ operation: () throws -> Void) { let p = currentPageIndex, s = pdfView.scaleFactor; perform { try operation(); refreshDocument(preservePage: p, scale: s) } }
    private func textPrompt(_ title: String, placeholder: String) -> String? { let field = NSTextField(string: ""); field.placeholderString = placeholder; field.setAccessibilityLabel(placeholder); field.frame = NSRect(x: 0, y: 0, width: 300, height: 24); let a = NSAlert(); a.messageText = title; a.accessoryView = field; a.addButton(withTitle: "确定"); a.addButton(withTitle: "取消"); return a.runModal() == .alertFirstButtonReturn ? field.stringValue : nil }
    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        let hasDocument = workspace.document != nil
        switch menuItem.action {
        case #selector(undo): return workspace.canUndo
        case #selector(redo): return workspace.canRedo
        case #selector(printDocument): return hasDocument && workspace.allowsPrinting
        case #selector(reduceFileSize(_:)):
            return hasDocument && workspace.allowsCopying && workspace.allowsDocumentAssembly && !workspace.isEncryptedSource && !isLocalToolRunning
        case #selector(saveDocument), #selector(saveAsDocument), #selector(saveCopy), #selector(saveFlattenedCopy), #selector(savePasswordProtectedCopy), #selector(extractPages), #selector(exportPlainText), #selector(exportAnnotationSummaryText), #selector(exportAnnotationSummaryMarkdown), #selector(exportAnnotationSummaryHTML):
            return hasDocument && workspace.allowsCopying
        case #selector(highlight), #selector(underline), #selector(strikeout), #selector(addFreeText), #selector(addSignature), #selector(toggleInk(_:)), #selector(selectEraser), #selector(addPopupNote), #selector(addRectangle), #selector(addEllipse), #selector(addLineAnnotation), #selector(addArrow), #selector(addDoubleArrow):
            return hasDocument && workspace.allowsCommenting
        case #selector(editSelectedAnnotation), #selector(deleteSelectedAnnotation):
            return hasDocument && workspace.allowsCommenting && annotationSelection.current(in: workspace) != nil
        case #selector(insertPDF), #selector(rotatePage), #selector(rotatePageLeft), #selector(insertBlank), #selector(movePageUp), #selector(deleteCurrentPage), #selector(duplicateCurrentPage), #selector(cropCurrentPage), #selector(addCurrentPageBookmark), #selector(renameTopLevelBookmark), #selector(removeTopLevelBookmark):
            return hasDocument && workspace.allowsDocumentAssembly
        case #selector(showOutline):
            return hasDocument
        default: return true
        }
    }
}

private extension Array {
    subscript(safe index: Index) -> Element? { indices.contains(index) ? self[index] : nil }
}

private final class ModeButton: NSButton { var mode: WorkbenchMode? }

@MainActor final class SignatureCanvas: NSView {
    var strokes: [[CGPoint]] = []
    private var current: [CGPoint] = []
    override var mouseDownCanMoveWindow: Bool { false }
    override var acceptsFirstResponder: Bool { true }
    override func mouseDown(with event: NSEvent) {
        guard bounds.width > 0, bounds.height > 0 else { return }
        window?.makeFirstResponder(self)
        current = [normalized(event)]; needsDisplay = true
    }
    override func mouseDragged(with event: NSEvent) {
        guard !current.isEmpty else { return }
        current.append(normalized(event)); needsDisplay = true
    }
    override func mouseUp(with event: NSEvent) {
        guard !current.isEmpty else { return }
        current.append(normalized(event)); strokes.append(current)
        current = []; needsDisplay = true
    }
    private func normalized(_ event: NSEvent) -> CGPoint {
        let p = convert(event.locationInWindow, from: nil)
        return CGPoint(x: min(max(p.x / bounds.width, 0), 1), y: min(max(p.y / bounds.height, 0), 1))
    }
    override func draw(_ dirtyRect: NSRect) {
        NSColor.white.setFill(); bounds.fill(); NSColor.black.setStroke()
        for stroke in strokes + (current.isEmpty ? [] : [current]) {
            guard let first = stroke.first else { continue }
            let path = NSBezierPath(); path.move(to: CGPoint(x: first.x * bounds.width, y: first.y * bounds.height))
            for point in stroke.dropFirst() { path.line(to: CGPoint(x: point.x * bounds.width, y: point.y * bounds.height)) }
            path.lineWidth = 2; path.lineCapStyle = .round; path.lineJoinStyle = .round; path.stroke()
        }
    }
}
@MainActor final class SignaturePanel { func run(window: NSWindow?) -> [[CGPoint]]? { guard window != nil else { return nil }; let canvas = SignatureCanvas(frame: NSRect(x: 0, y: 0, width: 420, height: 170)); canvas.setAccessibilityLabel("绘制图形签名"); let alert = NSAlert(); alert.messageText = "绘制图形签名"; alert.informativeText = "这会插入手写图形，不是数字证书。"; alert.accessoryView = canvas; alert.addButton(withTitle: "插入"); alert.addButton(withTitle: "取消"); guard alert.runModal() == .alertFirstButtonReturn else { return nil }; let strokes = canvas.strokes.filter { $0.count > 1 }; return strokes.isEmpty ? nil : strokes } }

private final class DocumentCommandMenuItem: NSMenuItem { var documentAction: Selector? }

@MainActor final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate, NSMenuItemValidation {
    var window: NSWindow!
    let controller: ApplicationShellController
    private var pendingOpen: [URL] = []
    private var externalOpenReady = false
    var onExternalOpenFailure: ((Error) -> Void)?
    private var terminationPending = false
    private weak var inkMenuItem: NSMenuItem?
    override init() {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!.appendingPathComponent("PDF Master", isDirectory: true)
        let profile = Bundle.main.bundleIdentifier?.contains(".validation") == true ? support.appendingPathComponent("Validation", isDirectory: true).appendingPathComponent(Bundle.main.bundleIdentifier!, isDirectory: true) : support
        let validationAccount: AccountStore?
        if let id = Bundle.main.bundleIdentifier, id.contains(".validation") {
            validationAccount = AccountStore(metadataURL: profile.appendingPathComponent("accounts-v1.json"),
                secretStorage: ProfileAccountSecretStorage(base: KeychainAccountSecretStorage(), namespace: id))
        } else { validationAccount = nil }
        controller = ApplicationShellController(recentStore: RecentDocumentStore(url: profile.appendingPathComponent("recent-documents.json")), accountStore: validationAccount)
        super.init()
    }
    init(controller: ApplicationShellController) {
        self.controller = controller
        super.init()
    }
    func applicationDidFinishLaunching(_ notification: Notification) {
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820), styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
        // AppDelegate owns and reuses this window for warm file opens and Dock
        // reopens. AppKit's default release-on-close over-releases it under ARC.
        window.isReleasedWhenClosed = false
        window.titleVisibility = .hidden; window.titlebarAppearsTransparent = true; window.isMovableByWindowBackground = true
        window.title = "PDF Master"; window.contentViewController = controller; window.contentMinSize = NSSize(width: 1180, height: 700); window.setContentSize(NSSize(width: 1280, height: 820)); window.minSize = window.frameRect(forContentRect: NSRect(origin: .zero, size: window.contentMinSize)).size; window.delegate = self; window.center(); window.makeKeyAndOrderFront(nil)
        window.appearance = NSAppearance(named: .darkAqua)
        controller.installWindowChrome(in: window)
        controller.onActiveWorkbenchChange = { [weak self] workbench in
            if let item = self?.inkMenuItem { workbench?.registerInkMenuItem(item); if workbench == nil { item.state = .off } }
            self?.updateReaderMenus()
        }
        installMenus(); finishExternalOpenStartup()
    }
    func application(_ sender: NSApplication, openFile filename: String) -> Bool {
        let url = filename.lowercased().hasPrefix("file:") ? URL(string: filename) : URL(fileURLWithPath: filename)
        guard let url else { reportExternalOpenError(CocoaError(.fileReadInvalidFileName)); return false }
        return receiveExternalDocuments([url])
    }
    func application(_ application: NSApplication, open urls: [URL]) {
        _ = receiveExternalDocuments(urls)
    }
    func application(_ sender: NSApplication, openFiles filenames: [String]) {
        var succeeded = true
        for filename in filenames { if !self.application(sender, openFile: filename) { succeeded = false } }
        sender.reply(toOpenOrPrint: succeeded ? .success : .failure)
    }
    func finishExternalOpenStartup() {
        externalOpenReady = true
        let queued = pendingOpen; pendingOpen.removeAll()
        _ = receiveExternalDocuments(queued)
    }
    @discardableResult private func receiveExternalDocuments(_ urls: [URL]) -> Bool {
        var succeeded = true
        for url in urls {
            guard url.isFileURL else { reportExternalOpenError(CocoaError(.fileReadUnsupportedScheme)); succeeded = false; continue }
            guard externalOpenReady else {
                if !pendingOpen.contains(where: { $0.standardizedFileURL == url.standardizedFileURL }) { pendingOpen.append(url) }
                continue
            }
            let scoped = url.startAccessingSecurityScopedResource()
            let opened = controller.open(url: url)
            if scoped { url.stopAccessingSecurityScopedResource() }
            if !opened, let error = controller.lastOpenError { reportExternalOpenError(error) }
            if !opened { succeeded = false }
        }
        if externalOpenReady, !urls.isEmpty { window?.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true) }
        return succeeded
    }
    private func reportExternalOpenError(_ error: Error) {
        if let onExternalOpenFailure { onExternalOpenFailure(error); return }
        let alert = NSAlert()
        alert.messageText = "无法打开文件"
        alert.informativeText = "\(error.localizedDescription)\n请确认文件已下载完成且仍在原位置；也可先保存到本机，再使用 PDF Master 打开。"
        alert.addButton(withTitle: "好")
        if let window, window.attachedSheet == nil { alert.beginSheetModal(for: window) }
        else { alert.runModal() }
    }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        window?.makeKeyAndOrderFront(nil)
        return true
    }
    func windowShouldClose(_ sender: NSWindow) -> Bool { controller.guardAllDirty() }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if terminationPending { return .terminateLater }
        guard controller.guardAllDirty() else { return .terminateCancel }
        guard LocalToolActivity.hasRunningJobs else { return .terminateNow }
        terminationPending = true
        Task {
            await LocalToolActivity.cancelAndWait()
            self.terminationPending = false
            sender.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }
    private func installMenus() {
        let main = NSMenu()

        let appItem = NSMenuItem()
        let appMenu = NSMenu(title: "PDF Master")
        appMenu.addItem(withTitle: "关于 PDF Master", action: #selector(showAbout), keyEquivalent: "")
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "退出 PDF Master", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = appMenu; main.addItem(appItem)

        let fileItem = NSMenuItem()
        let fileMenu = NSMenu(title: "文件")
        addItems([
            ("打开…", #selector(WorkbenchController.openDocument), "o"),
            ("新建", #selector(WorkbenchController.newDocument), "n"),
            ("另存为…", #selector(WorkbenchController.saveAsDocument), "S"),
            ("存储副本…", #selector(WorkbenchController.saveCopy), ""),
            ("存储打平副本…", #selector(WorkbenchController.saveFlattenedCopy), ""),
            ("存储密码保护副本…", #selector(WorkbenchController.savePasswordProtectedCopy), ""),
            ("压缩文件大小…", #selector(WorkbenchController.reduceFileSize(_:)), ""),
            ("导出纯文本…", #selector(WorkbenchController.exportPlainText), ""),
            ("导出批注摘要 TXT…", #selector(WorkbenchController.exportAnnotationSummaryText), ""),
            ("导出批注摘要 Markdown…", #selector(WorkbenchController.exportAnnotationSummaryMarkdown), ""),
            ("导出批注摘要 HTML…", #selector(WorkbenchController.exportAnnotationSummaryHTML), ""),
            ("附加文件并合并…", #selector(WorkbenchController.insertPDF), ""),
            ("提取页面…", #selector(WorkbenchController.extractPages), ""),
            ("打印…", #selector(WorkbenchController.printDocument), "p")
        ], to: fileMenu)
        fileItem.submenu = fileMenu; main.addItem(fileItem)
        let newTab = NSMenuItem(title: "新标签", action: #selector(newTabAction), keyEquivalent: "t"); newTab.target = self
        fileMenu.addItem(newTab)
        let closeTab = NSMenuItem(title: "关闭当前标签", action: #selector(closeTabAction), keyEquivalent: "w"); closeTab.target = self
        fileMenu.addItem(closeTab)
        fileMenu.addItem(.separator())
        let newCanvas = NSMenuItem(title: "新建无边记", action: #selector(newCanvasAction), keyEquivalent: "N"); newCanvas.target = self
        fileMenu.addItem(newCanvas)
        let openCanvas = NSMenuItem(title: "打开无边记…", action: #selector(openCanvasAction), keyEquivalent: ""); openCanvas.target = self
        fileMenu.addItem(openCanvas)
        let saveCanvas = NSMenuItem(title: "保存", action: #selector(saveCanvasAction), keyEquivalent: "s"); saveCanvas.target = self
        fileMenu.addItem(saveCanvas)
        let toCanvas = NSMenuItem(title: "将当前 PDF 复制到无边记", action: #selector(pdfToCanvasAction), keyEquivalent: ""); toCanvas.target = self
        fileMenu.addItem(toCanvas)

        let editItem = NSMenuItem()
        let editMenu = NSMenu(title: "编辑")
        addItems([
            ("撤销", #selector(WorkbenchController.undo), "z"),
            ("重做", #selector(WorkbenchController.redo), "Z")
        ], to: editMenu)
        DocumentMenuSelectors.addTextEditingItems(to: editMenu)
        addItems([
            ("高亮", #selector(WorkbenchController.highlight), ""),
            ("下划线", #selector(WorkbenchController.underline), ""),
            ("删除线", #selector(WorkbenchController.strikeout), ""),
            ("文字批注…", #selector(WorkbenchController.addFreeText), ""),
            ("图形签名…", #selector(WorkbenchController.addSignature), "")
        ], to: editMenu)
        let inkItem = DocumentCommandMenuItem(title: "手绘模式", action: #selector(runDocumentCommand(_:)), keyEquivalent: "")
        inkItem.documentAction = #selector(WorkbenchController.toggleInk(_:)); inkItem.target = self
        ExpertShortcuts.apply(to: inkItem, action: #selector(WorkbenchController.toggleInk(_:)))
        editMenu.addItem(inkItem)
        inkMenuItem = inkItem; controller.activeWorkbench?.registerInkMenuItem(inkItem)
        editItem.submenu = editMenu; main.addItem(editItem)

        let annotationItem = NSMenuItem()
        let annotationMenu = NSMenu(title: "批注")
        addItems([
            ("便筺…", #selector(WorkbenchController.addPopupNote), ""),
            ("矩形…", #selector(WorkbenchController.addRectangle), ""),
            ("椭圆…", #selector(WorkbenchController.addEllipse), ""),
            ("直线…", #selector(WorkbenchController.addLineAnnotation), ""),
            ("箭头…", #selector(WorkbenchController.addArrow), ""),
            ("双向箭头…", #selector(WorkbenchController.addDoubleArrow), ""),
            ("橡皮", #selector(WorkbenchController.selectEraser), ""),
            ("编辑选中批注…", #selector(WorkbenchController.editSelectedAnnotation), ""),
            ("删除选中批注…", #selector(WorkbenchController.deleteSelectedAnnotation), "")
        ], to: annotationMenu)
        annotationItem.submenu = annotationMenu; main.addItem(annotationItem)

        let pageItem = NSMenuItem()
        let pageMenu = NSMenu(title: "页面")
        addItems([
            ("页面预览", #selector(WorkbenchController.togglePageOverview), "4"),
            ("顺时针旋转", #selector(WorkbenchController.rotatePage), ""),
            ("逆时针旋转", #selector(WorkbenchController.rotatePageLeft), ""),
            ("插入空白页", #selector(WorkbenchController.insertBlank), ""),
            ("复制当前页", #selector(WorkbenchController.duplicateCurrentPage), ""),
            ("裁剪当前页…", #selector(WorkbenchController.cropCurrentPage), ""),
            ("移动当前页…", #selector(WorkbenchController.movePageUp), ""),
            ("删除页面…", #selector(WorkbenchController.deleteCurrentPage), "")
        ], to: pageMenu)
        pageItem.submenu = pageMenu; main.addItem(pageItem)

        let outlineItem = NSMenuItem()
        let outlineMenu = NSMenu(title: "大纲")
        addItems([
            ("查看与导航…", #selector(WorkbenchController.showOutline), ""),
            ("为当前页添加顶层书签…", #selector(WorkbenchController.addCurrentPageBookmark), ""),
            ("重命名顶层书签…", #selector(WorkbenchController.renameTopLevelBookmark), ""),
            ("删除顶层书签…", #selector(WorkbenchController.removeTopLevelBookmark), "")
        ], to: outlineMenu)
        outlineItem.submenu = outlineMenu; main.addItem(outlineItem)

        NSApp.mainMenu = main
        updateReaderMenus()
    }

    private func updateReaderMenus() {
        guard let main = NSApp.mainMenu else { return }
        for (identifier, title, commands) in [
            ("reader-view", "显示", [ReaderCommand.singlePage, .twoPages, .cover, .fitPage, .fitWidth, .actualSize, .zoomIn, .zoomOut, .outline, .thumbnails, .toggleSidebar, .nextTab, .previousTab, .closeOtherTabs]),
            ("reader-find", "查找", [ReaderCommand.find, .findNext, .findPrevious, .findSelection])
        ] {
            let id = NSUserInterfaceItemIdentifier(identifier)
            let item = main.items.first { $0.identifier == id } ?? NSMenuItem(title: title, action: nil, keyEquivalent: "")
            item.identifier = id
            item.submenu = controller.activeWorkbench?.readerCommands.makeMenu(commands: commands, title: title) ?? NSMenu(title: title)
            if item.menu == nil { main.addItem(item) }
        }
    }
    @objc private func newTabAction() { controller.showHome() }
    @objc private func closeTabAction() {
        guard NSApp.modalWindow == nil else { return }
        WindowCloseRouting.perform(keyWindow: NSApp.keyWindow, mainWindow: window) {
            if let id = controller.activeTabID { _ = controller.close(tabID: id) }
        }
    }
    @objc private func newCanvasAction() { controller.createCanvas() }
    @objc private func saveCanvasAction() {
        if let canvas = controller.activeCanvas { _ = canvas.saveBoard() }
        else { controller.activeWorkbench?.saveDocument() }
    }
    @objc private func openCanvasAction() {
        let panel = NSOpenPanel(); panel.allowedContentTypes = [UTType(exportedAs: "cn.nankong.pdfmaster.board", conformingTo: .data)]
        if panel.runModal() == .OK, let url = panel.url {
            _ = controller.openCanvas(url: url)
            if let error = controller.lastOpenError { let a = NSAlert(); a.messageText = "无法打开无边记"; a.informativeText = error.localizedDescription; a.runModal() }
        }
    }
    @objc private func pdfToCanvasAction() {
        do { try controller.sendActivePDFToCanvas() }
        catch { let a = NSAlert(); a.messageText = "无法复制 PDF 到画布"; a.informativeText = error.localizedDescription; a.runModal() }
    }

    private func addItems(_ items: [(String, Selector, String)], to menu: NSMenu) {
        for (title, action, key) in items {
            if action == DocumentMenuSelectors.copy || action == DocumentMenuSelectors.cut {
                menu.addItem(withTitle: title, action: action, keyEquivalent: key)
            } else {
                let item = DocumentCommandMenuItem(title: title, action: #selector(runDocumentCommand(_:)), keyEquivalent: key)
                ExpertShortcuts.apply(to: item, action: action)
                item.documentAction = action; item.target = self; menu.addItem(item)
            }
        }
    }
    @objc private func runDocumentCommand(_ item: DocumentCommandMenuItem) {
        guard let action = item.documentAction else { return }
        let isUndo = action == #selector(WorkbenchController.undo), isRedo = action == #selector(WorkbenchController.redo)
        if isUndo || isRedo {
            if TextHistoryRouting.perform(in: NSApp.keyWindow, redo: isRedo) { return }
            guard NSApp.modalWindow == nil, NSApp.keyWindow == nil || NSApp.keyWindow === window else { return }
        }
        if let canvas = controller.activeCanvas {
            if isUndo { canvas.undoAction(); return }
            if isRedo { canvas.redoAction(); return }
        }
        if action == #selector(WorkbenchController.openDocument) {
            let panel = NSOpenPanel(); panel.allowedContentTypes = [.pdf]
            if panel.runModal() == .OK, let url = panel.url {
                _ = controller.open(url: url)
                if let error = controller.lastOpenError {
                    let alert = NSAlert(); alert.messageText = "无法打开文档"; alert.informativeText = error.localizedDescription; alert.runModal()
                }
            }
        } else if action == #selector(WorkbenchController.newDocument) { controller.createBlankDocument() }
        else { _ = controller.routeDocumentCommand(action, sender: item) }
    }
    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        if item.action == #selector(closeTabAction) {
            guard NSApp.modalWindow == nil else { return false }
            if let key = NSApp.keyWindow, key !== window {
                item.title = "关闭窗口"
                return key.styleMask.contains(.closable)
            }
            item.title = "关闭当前标签"
            return controller.activeTabID != nil
        }
        if item.action == #selector(saveCanvasAction) { return controller.activeCanvas != nil || controller.activeWorkbench?.workspace.allowsCopying == true }
        if item.action == #selector(pdfToCanvasAction) { return controller.activeWorkbench?.workspace.allowsCopying == true }
        guard let routed = item as? DocumentCommandMenuItem, let action = routed.documentAction else { return true }
        if action == #selector(WorkbenchController.undo) || action == #selector(WorkbenchController.redo) {
            if let allowed = TextHistoryRouting.canPerform(in: NSApp.keyWindow, redo: action == #selector(WorkbenchController.redo)) { return allowed }
            guard NSApp.modalWindow == nil, NSApp.keyWindow == nil || NSApp.keyWindow === window else { return false }
        }
        if action == #selector(WorkbenchController.openDocument) || action == #selector(WorkbenchController.newDocument) { return true }
        if let canvas = controller.activeCanvas {
            if action == #selector(WorkbenchController.undo) { return canvas.session.canUndo }
            if action == #selector(WorkbenchController.redo) { return canvas.session.canRedo }
        }
        guard let workbench = controller.activeWorkbench else { return false }
        return workbench.validateMenuItem(NSMenuItem(title: item.title, action: action, keyEquivalent: ""))
    }
    @objc private func showAbout(){ let a=NSAlert(); a.messageText=Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "PDF Master"; a.informativeText="阅读、批注、页面整理与无限画布。\n文档在本机处理，AI 功能由你确认后连接服务。\n版本 V\(Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.1.7") · © Nankong"; a.runModal() }
}

MainActor.assumeIsolated {
    let app = NSApplication.shared
    let delegate = AppDelegate()
    app.delegate = delegate
    app.setActivationPolicy(.regular)
    app.run()
}
