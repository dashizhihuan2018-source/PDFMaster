import AppKit
import PDFKit
import UniformTypeIdentifiers
import ImageIO
import PaperCore

enum CanvasPDFPageSelection: Equatable {
    /// Human-facing, 1-based page number.
    case page(Int), allPages
}

/// AppKit owns the blur and appearance. A solid semantic surface is used when
/// the user requests reduced transparency; chrome never forces a dark theme.
@MainActor final class CanvasGlassCapsule: NSVisualEffectView {
    private let opaqueSurface = NSView()
    init(content: NSView) {
        super.init(frame: .zero)
        material = .popover; blendingMode = .withinWindow; state = .active
        wantsLayer = true; layer?.cornerRadius = 16; layer?.masksToBounds = false
        layer?.shadowOpacity = 0.10; layer?.shadowRadius = 8; layer?.shadowOffset = CGSize(width: 0, height: -2)
        opaqueSurface.wantsLayer = true; opaqueSurface.layer?.cornerRadius = 16
        opaqueSurface.translatesAutoresizingMaskIntoConstraints = false
        content.translatesAutoresizingMaskIntoConstraints = false
        addSubview(opaqueSurface); addSubview(content)
        NSLayoutConstraint.activate([
            opaqueSurface.leadingAnchor.constraint(equalTo: leadingAnchor), opaqueSurface.trailingAnchor.constraint(equalTo: trailingAnchor),
            opaqueSurface.topAnchor.constraint(equalTo: topAnchor), opaqueSurface.bottomAnchor.constraint(equalTo: bottomAnchor),
            content.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 9), content.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -9),
            content.topAnchor.constraint(equalTo: topAnchor, constant: 7), content.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -7)
        ])
        NSWorkspace.shared.notificationCenter.addObserver(self, selector: #selector(updateSurface), name: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil)
        updateSurface()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { NSWorkspace.shared.notificationCenter.removeObserver(self) }
    override var mouseDownCanMoveWindow: Bool { false }
    override func viewDidChangeEffectiveAppearance() { super.viewDidChangeEffectiveAppearance(); updateSurface() }
    @objc func updateSurface() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            opaqueSurface.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
            layer?.borderColor = NSColor.separatorColor.cgColor
            layer?.borderWidth = NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast ? 1 : 0.5
        }
        opaqueSurface.isHidden = !NSWorkspace.shared.accessibilityDisplayShouldReduceTransparency
    }
}

@MainActor final class CanvasSymbolButton: NSButton {
    private var pointerInside = false
    private var hoverTracking: NSTrackingArea?
    override var mouseDownCanMoveWindow: Bool { false }
    override var acceptsFirstResponder: Bool { isEnabled }
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let hoverTracking { removeTrackingArea(hoverTracking) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(area); hoverTracking = area
    }
    override func mouseEntered(with event: NSEvent) { pointerInside = true; needsDisplay = true }
    override func mouseExited(with event: NSEvent) { pointerInside = false; needsDisplay = true }
    override func becomeFirstResponder() -> Bool { needsDisplay = true; return super.becomeFirstResponder() }
    override func resignFirstResponder() -> Bool { needsDisplay = true; return super.resignFirstResponder() }
    override func draw(_ dirtyRect: NSRect) {
        let shape = NSBezierPath(roundedRect: bounds.insetBy(dx: 1, dy: 1), xRadius: 8, yRadius: 8)
        if isEnabled && (state == .on || isHighlighted || pointerInside) {
            (state == .on ? NSColor.controlAccentColor.withAlphaComponent(0.17) : NSColor.labelColor.withAlphaComponent(0.07)).setFill(); shape.fill()
        }
        contentTintColor = !isEnabled ? .tertiaryLabelColor : state == .on ? .controlAccentColor : .labelColor
        super.draw(dirtyRect)
        if window?.firstResponder === self {
            NSColor.keyboardFocusIndicatorColor.setStroke(); shape.lineWidth = 2; shape.stroke()
        }
    }
}

/// Independent native canvas document. The board file embeds its assets and never opens remote links.
@MainActor final class CanvasController: NSViewController {
    let session: CanvasSession
    let canvasView: CanvasView
    var sourceURL: URL?
    var onTitleChange: (() -> Void)?
    var onOpenPDF: ((Data,String) -> Void)?
    /// The shell attaches a real service-backed composer. Without that handler
    /// the visible AI entry is disabled, never a fake generation action.
    var onShowAI: (() -> Void)? { didSet { updateAIEntry() } }
    private let preferences: UserDefaults
    private var background: CanvasBackground
    private var backgroundButton: NSButton?
    private var aiButton: NSButton?
    private let titleField = NSTextField(string:"")
    private let statusLabel = NSTextField(labelWithString:"")
    private let zoomLabel = NSTextField(labelWithString:"100%")
    private let strokeWell = NSColorWell()
    private let fillWell = NSColorWell()
    private let widthField = NSTextField(string:"2")
    private var toolButtons:[NSButton] = []
    private var undoButton:NSButton?
    private var redoButton:NSButton?
    /// Tests and callers may supply a decision without displaying a modal. nil
    /// means cancellation and must leave the complete board/history untouched.
    var pdfPageSelectionPrompt: ((String, Int) -> CanvasPDFPageSelection?)?
    init(session: CanvasSession? = nil, preferences:UserDefaults = .standard) {
        let actualSession = session ?? CanvasSession()
        self.preferences = preferences
        background = CanvasBackground(rawValue:preferences.string(forKey:"PDFMaster.canvasBackground") ?? "light") ?? .light
        self.session = actualSession; canvasView = CanvasView(session:actualSession)
        canvasView.background = background
        super.init(nibName:nil,bundle:nil)
        actualSession.onChange = { [weak self] in self?.refresh() }
        canvasView.onSelectionChange = { [weak self] in self?.selectionChanged() }
        canvasView.onViewportChange = { [weak self] in self?.updateZoom() }
        canvasView.onOpenPDF = { [weak self] data,name in self?.onOpenPDF?(data,name) }
        canvasView.onError = { [weak self] error in self?.show(error) }
        canvasView.onDropFiles = { [weak self] urls, point in
            guard let self else { return false }
            do { return try self.importLocalFiles(urls, at: point) }
            catch { self.show(error); return false }
        }
    }
    required init?(coder:NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() {
        let root = NSView(); root.wantsLayer = true
        root.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor; view = root
        canvasView.translatesAutoresizingMaskIntoConstraints = false; root.addSubview(canvasView)
        NSLayoutConstraint.activate([
            canvasView.topAnchor.constraint(equalTo: root.topAnchor), canvasView.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            canvasView.leadingAnchor.constraint(equalTo: root.leadingAnchor), canvasView.trailingAnchor.constraint(equalTo: root.trailingAnchor)
        ])
        func row(_ children: [NSView] = []) -> NSStackView {
            let row = NSStackView(views: children); row.orientation = .horizontal; row.spacing = 3; row.alignment = .centerY
            return row
        }
        func capsule(_ content: NSView, id: String) -> CanvasGlassCapsule {
            let glass = CanvasGlassCapsule(content: content); glass.identifier = NSUserInterfaceItemIdentifier(id)
            glass.translatesAutoresizingMaskIntoConstraints = false; root.addSubview(glass); return glass
        }
        let tools = row()
        for tool in CanvasTool.allCases {
            let b = button(tool.title,symbol:tool.symbol,action:#selector(chooseTool(_:)))
            b.tag = tool.rawValue; b.setButtonType(.toggle); tools.addArrangedSubview(b); toolButtons.append(b)
        }
        tools.addArrangedSubview(button("导入图片",symbol:"photo",action:#selector(insertImage)))
        tools.addArrangedSubview(button("导入 PDF",symbol:"doc.richtext",action:#selector(insertPDF)))
        let undo = button("",symbol:"arrow.uturn.backward",action:#selector(undoAction)); undo.toolTip = "撤销 ⌘Z"
        let redo = button("",symbol:"arrow.uturn.forward",action:#selector(redoAction)); redo.toolTip = "重做 ⇧⌘Z"
        undo.setAccessibilityLabel("撤销"); redo.setAccessibilityLabel("重做")
        undoButton = undo; redoButton = redo
        let ai = button("AI创作",symbol:"sparkles",action:#selector(showAI)); aiButton = ai
        let actions = row([undo, redo, button("保存",symbol:"square.and.arrow.down",action:#selector(saveAction)), button("导出",symbol:"square.and.arrow.up",action:#selector(showExportMenu(_:))), ai])
        let settings = row()
        titleField.stringValue = session.board.title; titleField.font = .systemFont(ofSize:13,weight:.semibold)
        titleField.isBordered = false; titleField.drawsBackground = false; titleField.target = self; titleField.action = #selector(renameBoard)
        titleField.textColor = .labelColor; titleField.setAccessibilityLabel("画布名称")
        titleField.widthAnchor.constraint(equalToConstant:170).isActive = true
        let titleIcon = NSImageView(image: NSImage(systemSymbolName: "square.grid.2x2", accessibilityDescription: "画布")!)
        titleIcon.contentTintColor = .controlAccentColor
        let titleCapsule = capsule(row([titleIcon, titleField]), id: "canvas.title")
        strokeWell.color = CanvasView.color(canvasView.strokeHex); fillWell.color = CanvasView.color(canvasView.fillHex)
        for (label,well) in [("描边",strokeWell),("填充",fillWell)] {
            well.target = self; well.action = #selector(styleChanged); well.toolTip = label; well.setAccessibilityLabel(label)
            well.colorWellStyle = .minimal
            well.widthAnchor.constraint(equalToConstant:28).isActive = true; well.heightAnchor.constraint(equalToConstant:28).isActive = true
            settings.addArrangedSubview(well)
        }
        widthField.target = self; widthField.action = #selector(styleChanged); widthField.toolTip = "描边宽度（0.1–128）"; widthField.setAccessibilityLabel("描边宽度")
        widthField.font = .monospacedDigitSystemFont(ofSize:12, weight:.regular); widthField.alignment = .center
        widthField.widthAnchor.constraint(equalToConstant:40).isActive = true; settings.addArrangedSubview(widthField)
        let grid = button("显示点阵",symbol:"circle.grid.3x3.fill",action:#selector(toggleGrid(_:))); grid.setButtonType(.toggle); grid.state = .on
        let theme = button("切换黑白背景",symbol:"circle.lefthalf.filled",action:#selector(toggleBackground)); theme.setButtonType(.toggle); backgroundButton = theme
        zoomLabel.font = .monospacedDigitSystemFont(ofSize:12,weight:.medium); zoomLabel.textColor = .labelColor
        zoomLabel.alignment = .center; zoomLabel.widthAnchor.constraint(equalToConstant:52).isActive = true; zoomLabel.setAccessibilityLabel("画布缩放比例")
        let zoom = row([button("缩小",symbol:"minus",action:#selector(zoomOut)),zoomLabel,button("放大",symbol:"plus",action:#selector(zoomIn)),button("适合内容",symbol:"arrow.up.left.and.arrow.down.right",action:#selector(fitContent))])
        statusLabel.font = .systemFont(ofSize:11); statusLabel.textColor = .secondaryLabelColor
        statusLabel.toolTip = "滚轮／双指滚动缩放 · 双指捏合缩放 · Shift／空格／手掌平移 · 双击编辑／打开 PDF 副本 · 拖入本地图片或 PDF"
        zoomLabel.toolTip = "滚轮／双指滚动缩放 · 双指捏合缩放 · Shift／空格／手掌平移"
        let toolCapsule = capsule(tools, id: "canvas.tools"), styleCapsule = capsule(settings, id: "canvas.style")
        let actionCapsule = capsule(actions, id: "canvas.actions"), zoomCapsule = capsule(zoom, id: "canvas.zoom")
        let statusCapsule = capsule(row([statusLabel, grid, theme]), id: "canvas.status")
        NSLayoutConstraint.activate([
            titleCapsule.topAnchor.constraint(equalTo:root.topAnchor,constant:12),titleCapsule.leadingAnchor.constraint(equalTo:root.leadingAnchor,constant:14),
            actionCapsule.topAnchor.constraint(equalTo:root.topAnchor,constant:12),actionCapsule.trailingAnchor.constraint(equalTo:root.trailingAnchor,constant:-14),
            toolCapsule.topAnchor.constraint(equalTo:titleCapsule.bottomAnchor,constant:10),toolCapsule.leadingAnchor.constraint(equalTo:root.leadingAnchor,constant:14),
            styleCapsule.topAnchor.constraint(equalTo:toolCapsule.topAnchor),styleCapsule.trailingAnchor.constraint(equalTo:root.trailingAnchor,constant:-14),
            zoomCapsule.bottomAnchor.constraint(equalTo:root.bottomAnchor,constant:-14),zoomCapsule.leadingAnchor.constraint(equalTo:root.leadingAnchor,constant:14),
            statusCapsule.bottomAnchor.constraint(equalTo:root.bottomAnchor,constant:-14),statusCapsule.trailingAnchor.constraint(equalTo:root.trailingAnchor,constant:-14)
        ])
        applyBackground(); updateAIEntry(); refresh(); chooseTool(toolButtons[0])
    }
    private func applyBackground() {
        guard isViewLoaded else { return }
        // Appearance is scoped to this canvas only: other PDF tabs and the
        // application's appearance remain untouched.
        view.appearance = NSAppearance(named:background == .dark ? .darkAqua : .aqua)
        canvasView.background = background
        view.layer?.backgroundColor = canvasView.backgroundColor.cgColor
        backgroundButton?.state = background == .dark ? .on : .off
        backgroundButton?.toolTip = background == .dark ? "切换为白色背景（当前黑色）" : "切换为黑色背景（当前白色）"
        for glass in view.subviews.compactMap({ $0 as? CanvasGlassCapsule }) { glass.updateSurface() }
        view.needsDisplay = true
    }
    @objc private func toggleBackground() {
        background = background == .dark ? .light : .dark
        preferences.set(background.rawValue,forKey:"PDFMaster.canvasBackground"); applyBackground()
    }
    private func updateAIEntry() {
        aiButton?.isEnabled = onShowAI != nil
        aiButton?.toolTip = onShowAI == nil ? "AI创作（暂不可用）" : "AI创作"
    }
    @objc private func showAI() { onShowAI?() }
    private func button(_ title:String,symbol:String?,action:Selector) -> NSButton {
        let b = CanvasSymbolButton(title: title, target:self,action:action); b.bezelStyle = .inline; b.isBordered = false; b.focusRingType = .none
        b.controlSize = .regular; b.font = .systemFont(ofSize:13)
        b.toolTip = title; b.setAccessibilityLabel(title)
        b.widthAnchor.constraint(equalToConstant:30).isActive = true; b.heightAnchor.constraint(equalToConstant:30).isActive = true
        if let symbol { b.image = NSImage(systemSymbolName:symbol,accessibilityDescription:title); b.imagePosition = .imageOnly }
        return b
    }
    private func refresh() {
        canvasView.refresh(); guard isViewLoaded else { return }
        titleField.stringValue = session.board.title
        statusLabel.stringValue = "\(session.board.elements.count) 个对象 · \(session.isDirty ? "未保存" : "已保存")"
        undoButton?.isEnabled = session.canUndo; redoButton?.isEnabled = session.canRedo
        onTitleChange?()
    }
    private func updateZoom() { zoomLabel.stringValue = String(format:"%.0f%%",canvasView.viewport.scale*100) }
    private func selectionChanged() {
        guard let item = session.board.elements.first(where:{$0.id == canvasView.selectedID}) else { return }
        strokeWell.color = CanvasView.color(item.strokeHex); fillWell.color = CanvasView.color(item.fillHex); widthField.stringValue = String(format:"%.1f",item.lineWidth)
        canvasView.strokeHex = item.strokeHex; canvasView.fillHex = item.fillHex; canvasView.lineWidth = item.lineWidth
    }
    @objc private func chooseTool(_ sender:NSButton) {
        guard let tool = CanvasTool(rawValue:sender.tag) else { return }
        canvasView.tool = tool; for button in toolButtons { button.state = button.tag == tool.rawValue ? .on : .off }
        view.window?.makeFirstResponder(canvasView)
    }
    @objc private func toggleGrid(_ sender:NSButton) { canvasView.showsDotGrid = sender.state == .on }
    @objc private func zoomIn() { canvasView.zoom(to:canvasView.viewport.scale*1.25) }
    @objc private func zoomOut() { canvasView.zoom(to:canvasView.viewport.scale/1.25) }
    @objc private func fitContent() { canvasView.fitContent() }
    @objc private func undoAction() { canvasView.finishTextEditing(); canvasView.cancelGesture(); session.undo() }
    @objc private func redoAction() { canvasView.finishTextEditing(); canvasView.cancelGesture(); session.redo() }
    @objc private func renameBoard() {
        do { try session.mutate { $0.title = titleField.stringValue.isEmpty ? "未命名画布" : titleField.stringValue } } catch { show(error) }
    }
    private func hex(_ color:NSColor) -> String {
        guard let c = color.usingColorSpace(.sRGB) else { return "#26334D" }
        return String(format:"#%02X%02X%02X%02X",Int(round(c.redComponent*255)),Int(round(c.greenComponent*255)),Int(round(c.blueComponent*255)),Int(round(c.alphaComponent*255)))
    }
    @objc private func styleChanged() {
        guard let width = Double(widthField.stringValue),width >= 0.1,width <= 128,width.isFinite else {
            widthField.stringValue = String(format:"%.1f",canvasView.lineWidth); NSSound.beep(); return
        }
        canvasView.applyStyle(stroke:hex(strokeWell.color),fill:hex(fillWell.color),width:width)
    }
    @objc private func insertImage() { chooseAsset(kind:.image) }
    @objc private func insertPDF() { chooseAsset(kind:.pdf) }
    @objc private func showExportMenu(_ sender: NSButton) {
        let menu = NSMenu()
        for (title, action) in [("导出为 PDF", #selector(exportPDF)), ("导出为 PNG", #selector(exportPNG))] {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: ""); item.target = self; menu.addItem(item)
        }
        menu.popUp(positioning: nil, at: CGPoint(x: 0, y: sender.bounds.maxY), in: sender)
    }
    private func chooseAsset(kind:CanvasElementKind) {
        let panel = NSOpenPanel(); panel.canChooseDirectories = false; panel.allowsMultipleSelection = true
        panel.allowedContentTypes = kind == .pdf ? [.pdf] : [.png,.jpeg,.tiff,.heic]
        panel.message = kind == .pdf ? "PDF 将作为独立副本嵌入画布，不会修改原文件。" : "图片将嵌入画布文件。"
        guard panel.runModal() == .OK else { return }
        do { _ = try importLocalFiles(panel.urls) } catch { show(error) }
    }
    func importAsset(data:Data,name:String,kind:CanvasElementKind, selection:CanvasPDFPageSelection = .page(1), at point:CGPoint? = nil) throws {
        let items = try preparedAssets(data:data,name:name,kind:kind,selection:selection)
        try commitAssets(items, at:point)
    }
    /// Local Finder drops and the open panel share one transaction. Admission,
    /// page choices and resource budgets all complete before touching the board.
    @discardableResult func importLocalFiles(_ urls:[URL], at point:CGPoint? = nil) throws -> Bool {
        guard !urls.isEmpty, urls.count <= 500 else { throw CanvasBoardError.resourceLimit }
        var pending:[CanvasElement] = []
        var inputBytes = 0
        let existingBytes = session.board.elements.reduce(0) { $0 + ($1.assetData?.count ?? 0) }
        for url in urls {
            guard CanvasView.acceptsLocalAssetURL(url) else { throw CanvasBoardError.localFileRequired }
            let info = try url.resourceValues(forKeys:[.isRegularFileKey,.fileSizeKey])
            guard info.isRegularFile == true,let size = info.fileSize,size > 0 else { throw CanvasBoardError.invalidFile }
            guard size <= CanvasBoard.maxAssetBytes else { throw CanvasBoardError.resourceLimit }
            inputBytes += size
            guard inputBytes + existingBytes <= CanvasBoard.maxFileBytes else { throw CanvasBoardError.resourceLimit }
            let data = try Data(contentsOf:url,options:.mappedIfSafe)
            let kind:CanvasElementKind = url.pathExtension.lowercased() == "pdf" ? .pdf : .image
            var selection:CanvasPDFPageSelection = .page(1)
            if kind == .pdf {
                let admitted = PDFWorkspace(); try admitted.createUnsavedPDFCopy(data:data)
                let count = admitted.document?.pageCount ?? 0
                guard let choice = requestPDFPages(name:url.lastPathComponent,count:count) else { return false }
                selection = choice
            }
            let prepared = try preparedAssets(data:data,name:url.lastPathComponent,kind:kind,selection:selection)
            guard pending.count + prepared.count <= 500 else { throw CanvasBoardError.resourceLimit }
            pending.append(contentsOf:prepared)
            guard pending.reduce(existingBytes, { $0 + ($1.assetData?.count ?? 0) }) <= CanvasBoard.maxFileBytes else { throw CanvasBoardError.resourceLimit }
        }
        try commitAssets(pending,at:point); return true
    }
    private func requestPDFPages(name:String,count:Int) -> CanvasPDFPageSelection? {
        if let pdfPageSelectionPrompt { return pdfPageSelectionPrompt(name,count) }
        guard count > 1 else { return .page(1) }
        let alert = NSAlert(); alert.messageText = "将 PDF 页面加入画布"
        alert.informativeText = "\(name) · 共 \(count) 页。每页嵌入独立副本；不修改原文件。整批导入可一次撤销。"
        alert.addButton(withTitle:"导入"); alert.addButton(withTitle:"取消")
        let mode = NSPopUpButton(frame:CGRect(x:0,y:40,width:260,height:28)); mode.addItems(withTitles:["仅指定页","全部 \(count) 页"])
        mode.setAccessibilityLabel("PDF 导入范围")
        let page = NSTextField(string:"1"); page.frame = CGRect(x:100,y:3,width:70,height:26); page.setAccessibilityLabel("页码（从 1 开始）")
        let label = NSTextField(labelWithString:"页码（1–\(count)）"); label.frame = CGRect(x:0,y:6,width:100,height:20)
        let accessory = NSView(frame:CGRect(x:0,y:0,width:260,height:72)); accessory.addSubview(mode); accessory.addSubview(label); accessory.addSubview(page)
        alert.accessoryView = accessory
        guard alert.runModal() == .alertFirstButtonReturn else { return nil }
        if mode.indexOfSelectedItem == 1 { return .allPages }
        return .page(Int(page.stringValue.trimmingCharacters(in:.whitespacesAndNewlines)) ?? 0)
    }
    private func preparedAssets(data:Data,name:String,kind:CanvasElementKind,selection:CanvasPDFPageSelection) throws -> [CanvasElement] {
        guard !data.isEmpty,kind == .image || kind == .pdf else { throw CanvasBoardError.invalidAsset }
        guard data.count <= CanvasBoard.maxAssetBytes else { throw CanvasBoardError.resourceLimit }
        var payloads:[(Data,CGSize,String)] = []
        if kind == .pdf {
            let admission = PDFWorkspace()
            try admission.createUnsavedPDFCopy(data:data)
            guard let document = admission.document,(1...500).contains(document.pageCount) else { throw CanvasBoardError.invalidAsset }
            let indices:[Int]
            switch selection {
            case .page(let number):
                guard (1...document.pageCount).contains(number) else { throw CanvasBoardError.invalidAsset }
                indices = [number-1]
            case .allPages: indices = Array(0..<document.pageCount)
            }
            for index in indices {
                guard let page = document.page(at:index) else { throw CanvasBoardError.invalidAsset }
                let dimensions = page.bounds(for:.cropBox).size
                guard dimensions.width.isFinite,dimensions.height.isFinite,dimensions.width > 0,dimensions.height > 0 else { throw CanvasBoardError.invalidGeometry }
                let size = CGSize(width:240,height:min(420,max(120,240*dimensions.height/dimensions.width)))
                let bytes:Data
                if document.pageCount == 1 { bytes = data }
                else {
                    let single = PDFDocument()
                    guard let copied = page.copy() as? PDFPage else { throw CanvasBoardError.invalidAsset }
                    single.insert(copied,at:0)
                    guard let serialized = single.dataRepresentation() else { throw CanvasBoardError.invalidAsset }
                    // Fail closed if normalization produced an inadmissible attachment.
                    let check = PDFWorkspace(); try check.createUnsavedPDFCopy(data:serialized)
                    guard check.document?.pageCount == 1 else { throw CanvasBoardError.invalidAsset }
                    bytes = serialized
                }
                let label = document.pageCount > 1 ? String(name.prefix(220))+" · 第 \(index+1) 页" : String(name.prefix(250))
                payloads.append((bytes,size,label))
                guard payloads.reduce(0, { $0 + $1.0.count }) <= CanvasBoard.maxFileBytes else { throw CanvasBoardError.resourceLimit }
            }
        } else {
            guard let source = CGImageSourceCreateWithData(data as CFData,[kCGImageSourceShouldCache:false] as CFDictionary),
                  let properties = CGImageSourceCopyPropertiesAtIndex(source,0,nil) as? [CFString:Any],
                  let w = properties[kCGImagePropertyPixelWidth] as? NSNumber,let h = properties[kCGImagePropertyPixelHeight] as? NSNumber,
                  w.doubleValue > 0,h.doubleValue > 0,w.doubleValue <= 32768,h.doubleValue <= 32768,w.doubleValue*h.doubleValue <= 100_000_000 else { throw CanvasBoardError.invalidAsset }
            let scale = min(1,360/max(w.doubleValue,h.doubleValue)); let size = CGSize(width:w.doubleValue*scale,height:h.doubleValue*scale)
            payloads.append((data,size,String(name.prefix(250))))
        }
        return payloads.map { bytes,size,label in
            CanvasElement(kind:kind,frame:CGRect(origin:.zero,size:size),strokeHex:"#CED4DE",fillHex:"#FFFFFF",assetData:bytes,assetName:label)
        }
    }
    private func commitAssets(_ items:[CanvasElement], at point:CGPoint?) throws {
        guard !items.isEmpty,items.count <= 500 else { throw CanvasBoardError.resourceLimit }
        let center = point ?? canvasView.viewport.worldPoint(from:CGPoint(x:canvasView.bounds.midX,y:canvasView.bounds.midY))
        guard center.x.isFinite,center.y.isFinite else { throw CanvasBoardError.invalidGeometry }
        let columns = min(4,Int(ceil(sqrt(Double(items.count)))))
        let cellWidth = (items.map { $0.frame.width }.max() ?? 240)+24
        let cellHeight = (items.map { $0.frame.height }.max() ?? 320)+24
        let positioned = items.enumerated().map { index,item -> CanvasElement in
            var placed = item
            placed.frame.origin = CGPoint(x:center.x-item.frame.width/2+CGFloat(index%columns)*cellWidth,
                                          y:center.y-item.frame.height/2+CGFloat(index/columns)*cellHeight)
            return placed
        }
        // No editing or gesture changes until the complete candidate is valid.
        var candidate = session.board; candidate.elements.append(contentsOf:positioned); _ = try candidate.validated()
        canvasView.cancelGesture()
        try session.mutate { $0.elements.append(contentsOf:positioned) }; canvasView.select(positioned.first?.id)
    }
    /// Admission before switching tabs or committing any active editor. A malformed
    /// embedded resource cannot replace the current document, even if its JSON is valid.
    static func validatedAssets(_ board:CanvasBoard) throws -> CanvasBoard {
        _ = try board.validated()
        for item in board.elements {
            guard item.kind == .pdf || item.kind == .image else { continue }
            guard let data = item.assetData else { throw CanvasBoardError.invalidAsset }
            if item.kind == .pdf {
                let admission = PDFWorkspace()
                try admission.createUnsavedPDFCopy(data:data)
                guard let doc = PDFDocument(data:data),!doc.isEncrypted,!doc.isLocked,doc.pageCount > 0 else { throw CanvasBoardError.invalidAsset }
                guard let page = doc.page(at:0),page.bounds(for:.cropBox).width > 0,page.bounds(for:.cropBox).height > 0 else { throw CanvasBoardError.invalidGeometry }
            } else {
                guard let source = CGImageSourceCreateWithData(data as CFData,[kCGImageSourceShouldCache:false] as CFDictionary),
                    let properties = CGImageSourceCopyPropertiesAtIndex(source,0,nil) as? [CFString:Any],
                    let w = properties[kCGImagePropertyPixelWidth] as? NSNumber,let h = properties[kCGImagePropertyPixelHeight] as? NSNumber,
                    w.doubleValue > 0,h.doubleValue > 0,w.doubleValue <= 32768,h.doubleValue <= 32768,w.doubleValue*h.doubleValue <= 100_000_000 else { throw CanvasBoardError.invalidAsset }
            }
        }
        return board
    }
    func finishEditing() {
        let pendingTitle = isViewLoaded ? ((titleField.currentEditor() as? NSTextView)?.string ?? titleField.stringValue) : session.board.title
        canvasView.finishTextEditing()
        if pendingTitle != session.board.title {
            do { try session.mutate { $0.title = pendingTitle.isEmpty ? "未命名画布" : pendingTitle } } catch { show(error) }
        }
    }
    @discardableResult func saveBoard() -> Bool {
        finishEditing(); canvasView.cancelGesture()
        var destination = sourceURL
        if destination == nil {
            let panel = NSSavePanel(); panel.allowedContentTypes = [UTType(filenameExtension:"pdfmasterboard") ?? .json]
            panel.nameFieldStringValue = session.board.title+".pdfmasterboard"
            guard panel.runModal() == .OK,let url = panel.url else { return false }; destination = url
        }
        do { try session.save(to:destination!); sourceURL = destination; onTitleChange?(); return true } catch { show(error); return false }
    }
    func openBoard(url:URL) throws {
        let board = try Self.validatedAssets(CanvasBoard.load(from:url))
        canvasView.finishTextEditing(); canvasView.cancelGesture(); try session.replaceBoard(board)
        sourceURL = url; canvasView.select(nil); canvasView.fitContent(); onTitleChange?()
    }
    func guardDirty() -> Bool {
        finishEditing(); guard session.isDirty else { return true }
        let alert = NSAlert(); alert.messageText = "保存画布更改？"; alert.informativeText = "关闭未保存的画布会丢失更改。"
        alert.addButton(withTitle:"保存"); alert.addButton(withTitle:"不保存"); alert.addButton(withTitle:"取消")
        switch alert.runModal() { case .alertFirstButtonReturn:return saveBoard(); case .alertSecondButtonReturn:return true; default:return false }
    }
    @objc private func saveAction() { _ = saveBoard() }
    @objc private func exportPDF() { exportData(canvasView.exportPDFData(),extension:"pdf",type:.pdf) }
    @objc private func exportPNG() { canvasView.finishTextEditing(); canvasView.cancelGesture(); exportData(canvasView.renderPNGData(allContent:true),extension:"png",type:.png) }
    private func exportData(_ data:Data?,extension ext:String,type:UTType) {
        guard let data else { show(CanvasBoardError.resourceLimit); return }
        let panel = NSSavePanel(); panel.allowedContentTypes = [type]; panel.nameFieldStringValue = session.board.title+"."+ext
        guard panel.runModal() == .OK,let url = panel.url else { return }
        do { try data.write(to:url,options:.atomic) } catch { show(error) }
    }
    private func show(_ error:Error) {
        let alert = NSAlert(error:error); alert.runModal()
    }
}
