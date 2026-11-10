import AppKit
import ImageIO
import PaperCore
import UniformTypeIdentifiers

/// An uncommitted, normalized image snapshot belongs to this editor alone.
@MainActor final class SignatureEditorController: NSViewController, NSWindowDelegate {
    enum Mode { case image, draw }
    private let store: SignatureAssetStore
    private let onCreate: (SignatureAsset) -> Void
    private let chooseFile: (() -> URL?)?
    private let presentError: @MainActor (String) -> Void
    private(set) var draft: SignatureImage?
    private(set) var mode: Mode = .image
    private(set) var editorWindow: NSWindow?
    let dropView = SignatureDropView()
    let preview = NSImageView()
    let canvas = SignatureCanvas()
    private let prompt = NSTextField(labelWithString: "拖放图片即新建签名")
    private let modes = NSSegmentedControl(labels: ["图片", "绘制"], trackingMode: .selectOne, target: nil, action: nil)

    init(store: SignatureAssetStore, onCreate: @escaping (SignatureAsset) -> Void,
         chooseFile: (() -> URL?)? = nil,
         presentError: @escaping @MainActor (String) -> Void = { message in
             let alert = NSAlert(); alert.messageText = "无法创建签名"; alert.informativeText = message; alert.runModal()
         }) {
        self.store = store; self.onCreate = onCreate; self.chooseFile = chooseFile; self.presentError = presentError
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 480, height: 408))
        modes.target = self; modes.action = #selector(modeAction); modes.selectedSegment = 0
        modes.setAccessibilityLabel("签名创建方式")
        dropView.registerForDraggedTypes([.fileURL]); dropView.onFile = { [weak self] url in self?.acceptImage(from: url) ?? false }
        preview.imageScaling = .scaleProportionallyUpOrDown; preview.setAccessibilityLabel("签名图片预览")
        let choose = NSButton(title: "选择文件…", target: self, action: #selector(chooseAction))
        choose.font = .systemFont(ofSize: 12, weight: .medium); choose.setAccessibilityLabel("选择 PNG 签名图片")
        prompt.alignment = .center
        [preview, prompt, choose].forEach { $0.translatesAutoresizingMaskIntoConstraints = false; dropView.addSubview($0) }
        let cancel = NSButton(title: "取消", target: self, action: #selector(cancelAction)); cancel.keyEquivalent = "\u{1b}"
        let create = NSButton(title: "创建", target: self, action: #selector(createAction)); create.keyEquivalent = "\r"
        let clear = NSButton(title: "清除", target: self, action: #selector(clearDrawing)); clear.identifier = NSUserInterfaceItemIdentifier("clearDrawing")
        canvas.setAccessibilityLabel("绘制签名"); canvas.wantsLayer = true; canvas.layer?.borderColor = NSColor.separatorColor.cgColor; canvas.layer?.borderWidth = 1
        [modes, dropView, canvas, clear, cancel, create].forEach { $0.translatesAutoresizingMaskIntoConstraints = false; view.addSubview($0) }
        NSLayoutConstraint.activate([
            modes.centerXAnchor.constraint(equalTo: view.centerXAnchor), modes.topAnchor.constraint(equalTo: view.topAnchor, constant: 16),
            dropView.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 20), dropView.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -20),
            dropView.topAnchor.constraint(equalTo: modes.bottomAnchor, constant: 16), dropView.bottomAnchor.constraint(equalTo: cancel.topAnchor, constant: -20),
            prompt.centerXAnchor.constraint(equalTo: dropView.centerXAnchor), prompt.centerYAnchor.constraint(equalTo: dropView.centerYAnchor, constant: -24),
            choose.centerXAnchor.constraint(equalTo: dropView.centerXAnchor), choose.bottomAnchor.constraint(equalTo: dropView.bottomAnchor, constant: -12),
            choose.heightAnchor.constraint(equalToConstant: 28), choose.widthAnchor.constraint(greaterThanOrEqualToConstant: 132),
            preview.topAnchor.constraint(equalTo: dropView.topAnchor, constant: 8), preview.leadingAnchor.constraint(equalTo: dropView.leadingAnchor, constant: 12),
            preview.trailingAnchor.constraint(equalTo: dropView.trailingAnchor, constant: -12), preview.bottomAnchor.constraint(equalTo: choose.topAnchor, constant: -12),
            canvas.leadingAnchor.constraint(equalTo: dropView.leadingAnchor), canvas.trailingAnchor.constraint(equalTo: dropView.trailingAnchor),
            canvas.topAnchor.constraint(equalTo: dropView.topAnchor), canvas.bottomAnchor.constraint(equalTo: dropView.bottomAnchor, constant: -36),
            clear.leadingAnchor.constraint(equalTo: canvas.leadingAnchor), clear.topAnchor.constraint(equalTo: canvas.bottomAnchor, constant: 4),
            cancel.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -16), create.centerYAnchor.constraint(equalTo: cancel.centerYAnchor),
            create.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -20), cancel.trailingAnchor.constraint(equalTo: create.leadingAnchor, constant: -12),
            cancel.widthAnchor.constraint(greaterThanOrEqualToConstant: 72), create.widthAnchor.constraint(greaterThanOrEqualToConstant: 72)
        ])
        updateMode()
    }
    func present(from owner: NSWindow) {
        guard editorWindow == nil, owner.attachedSheet == nil else { return }
        resetDraft()
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 480, height: 408), styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.title = "新建签名"; window.isReleasedWhenClosed = false; window.contentViewController = self; window.delegate = self
        editorWindow = window; owner.beginSheet(window)
    }
    func resetDraft() {
        _ = view; draft = nil; preview.image = nil; prompt.isHidden = false
        canvas.strokes = []; canvas.needsDisplay = true; mode = .image; modes.selectedSegment = 0; updateMode()
    }
    func setMode(_ mode: Mode) { _ = view; self.mode = mode; modes.selectedSegment = mode == .image ? 0 : 1; updateMode() }
    private func updateMode() {
        dropView.isHidden = mode != .image; canvas.isHidden = mode != .draw
        view.subviews.first { $0.identifier?.rawValue == "clearDrawing" }?.isHidden = mode != .draw
    }
    func chooseImage() {
        if let chooseFile { if let url = chooseFile() { _ = acceptImage(from: url) }; return }
        guard let owner = editorWindow else { return }
        let open = NSOpenPanel(); open.allowedContentTypes = [.png]; open.allowsMultipleSelection = false; open.canChooseDirectories = false
        open.beginSheetModal(for: owner) { [weak self] result in
            if result == .OK, let url = open.url { _ = self?.acceptImage(from: url) }
        }
    }
    @discardableResult func acceptImage(from url: URL) -> Bool {
        _ = view
        do {
            let image = try store.draft(from: url)
            guard let nsImage = NSImage(data: image.pngData) else { throw CocoaError(.fileReadCorruptFile) }
            // Only bytes survive acceptance; no source URL is retained.
            draft = image; preview.image = nsImage; prompt.isHidden = true; setMode(.image)
            return true
        } catch { presentError(error.localizedDescription); return false }
    }
    @discardableResult func create() -> Bool {
        do {
            let image: SignatureImage
            if mode == .draw { image = try drawnImage() }
            else { guard let draft else { presentError("请先选择 PNG 图片或绘制签名。"); return false }; image = draft }
            let asset = try store.save(image)
            finish(); resetDraft(); onCreate(asset); return true
        } catch { presentError(error.localizedDescription); return false }
    }
    private func drawnImage() throws -> SignatureImage {
        view.layoutSubtreeIfNeeded()
        let width = Int(canvas.bounds.width * 2), height = Int(canvas.bounds.height * 2)
        let strokes = canvas.strokes.filter { $0.count > 1 }
        guard width > 0, height > 0, width <= 4_000_000 / height, !strokes.isEmpty,
              strokes.flatMap({ $0 }).allSatisfy({ $0.x.isFinite && $0.y.isFinite }),
              let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { throw CocoaError(.validationMissingMandatoryProperty) }
        context.setStrokeColor(NSColor.black.cgColor); context.setLineWidth(4); context.setLineCap(.round); context.setLineJoin(.round)
        for stroke in strokes {
            context.beginPath(); context.move(to: CGPoint(x: stroke[0].x * CGFloat(width), y: stroke[0].y * CGFloat(height)))
            for point in stroke.dropFirst() { context.addLine(to: CGPoint(x: point.x * CGFloat(width), y: point.y * CGFloat(height))) }
            context.strokePath()
        }
        guard let image = context.makeImage(), let bytes = CFDataCreateMutable(nil, 0),
              let destination = CGImageDestinationCreateWithData(bytes, UTType.png.identifier as CFString, 1, nil) else { throw CocoaError(.fileWriteUnknown) }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else { throw CocoaError(.fileWriteUnknown) }
        return try SignatureImage(pngData: bytes as Data)
    }
    private func finish() {
        guard let window = editorWindow else { return }
        if let owner = window.sheetParent { owner.endSheet(window) }
        window.orderOut(nil); window.delegate = nil; window.contentViewController = nil; editorWindow = nil
    }
    func cancel() { finish(); resetDraft() }
    func windowShouldClose(_ sender: NSWindow) -> Bool { cancel(); return true }
    @objc private func modeAction() { setMode(modes.selectedSegment == 0 ? .image : .draw) }
    @objc private func chooseAction() { chooseImage() }
    @objc private func createAction() { _ = create() }
    @objc private func cancelAction() { cancel() }
    @objc private func clearDrawing() { canvas.strokes = []; canvas.needsDisplay = true }
}

@MainActor final class SignatureDropView: NSView {
    var onFile: ((URL) -> Bool)?
    private func file(_ sender: NSDraggingInfo) -> URL? {
        let files = sender.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL]
        guard let files, files.count == 1 else { return nil }; return files[0]
    }
    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation { file(sender) == nil ? [] : .copy }
    override func prepareForDragOperation(_ sender: NSDraggingInfo) -> Bool { file(sender) != nil }
    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool { file(sender).map { onFile?($0) ?? false } ?? false }
}
