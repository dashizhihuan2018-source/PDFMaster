import AppKit
import ImageIO
import PaperCore
import UniformTypeIdentifiers

@MainActor final class SignaturePickerController: NSObject {
    let store: SignatureAssetStore
    private let onSelectImage: (SignatureImage) -> Void
    private let onDraw: () -> Void
    private let chooseFile: @MainActor () -> URL?
    private let confirmDelete: @MainActor () -> Bool
    private let presentError: @MainActor (String) -> Void
    private var panel: NSPanel?
    private let list = NSStackView()
    var entries: [SignatureAsset] { store.entries.reversed() }

    init(store: SignatureAssetStore, onSelectImage: @escaping (SignatureImage) -> Void, onDraw: @escaping () -> Void,
         chooseFile: @escaping @MainActor () -> URL? = {
             let open = NSOpenPanel(); open.allowedContentTypes = [.png]; open.allowsMultipleSelection = false
             open.canChooseDirectories = false; return open.runModal() == .OK ? open.url : nil
         }, confirmDelete: @escaping @MainActor () -> Bool = {
             let alert = NSAlert(); alert.messageText = "从签名库删除？"
             alert.informativeText = "仅删除本机签名库中的副本，不会删除原始图片，也不会影响已签署的 PDF。"
             alert.addButton(withTitle: "删除"); alert.addButton(withTitle: "取消")
             return alert.runModal() == .alertFirstButtonReturn
         }, presentError: @escaping @MainActor (String) -> Void = { message in
             let alert = NSAlert(); alert.alertStyle = .warning; alert.messageText = "签名库操作失败"
             alert.informativeText = message; alert.runModal()
         }) {
        self.store = store; self.onSelectImage = onSelectImage; self.onDraw = onDraw
        self.chooseFile = chooseFile; self.confirmDelete = confirmDelete; self.presentError = presentError
    }
    func reloadHistory() { do { try store.reload(); render() } catch { presentError(error.localizedDescription) } }
    func importPNG() {
        guard let url = chooseFile() else { return }
        do { _ = try store.importPNG(from: url); render() } catch { presentError(error.localizedDescription) }
    }
    func select(id: UUID) {
        do { let image = try store.image(for: id); panel?.close(); onSelectImage(image) }
        catch { presentError(error.localizedDescription) }
    }
    func delete(id: UUID) {
        guard confirmDelete() else { return }
        do { try store.remove(id: id); render() } catch { presentError(error.localizedDescription) }
    }
    func show() {
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 430, height: 440), styleMask: [.titled, .closable], backing: .buffered, defer: false)
        panel.title = "我的签名"; panel.isReleasedWhenClosed = false
        let root = NSStackView(); root.orientation = .vertical; root.alignment = .leading; root.spacing = 10
        root.edgeInsets = NSEdgeInsets(top: 16, left: 16, bottom: 16, right: 16)
        let actions = NSStackView(); actions.orientation = .horizontal; actions.spacing = 12
        let importButton = NSButton(title: "导入 PNG…", target: self, action: #selector(importAction)); importButton.setAccessibilityLabel("导入 PNG 签名")
        let drawButton = NSButton(title: "手绘签名", target: self, action: #selector(drawAction)); drawButton.setAccessibilityLabel("手绘签名")
        actions.addArrangedSubview(importButton); actions.addArrangedSubview(drawButton); root.addArrangedSubview(actions)
        let scroll = NSScrollView(); scroll.hasVerticalScroller = true; scroll.drawsBackground = false
        list.orientation = .vertical; list.alignment = .leading; list.spacing = 8
        scroll.documentView = list; root.addArrangedSubview(scroll)
        root.translatesAutoresizingMaskIntoConstraints = false; panel.contentView?.addSubview(root)
        if let content = panel.contentView { NSLayoutConstraint.activate([root.leadingAnchor.constraint(equalTo: content.leadingAnchor), root.trailingAnchor.constraint(equalTo: content.trailingAnchor), root.topAnchor.constraint(equalTo: content.topAnchor), root.bottomAnchor.constraint(equalTo: content.bottomAnchor), scroll.widthAnchor.constraint(equalTo: root.widthAnchor, constant: -32), scroll.heightAnchor.constraint(greaterThanOrEqualToConstant: 300)]) }
        self.panel = panel; reloadHistory(); panel.center(); panel.makeKeyAndOrderFront(nil)
    }
    private func render() {
        guard panel != nil else { return }
        list.arrangedSubviews.forEach { list.removeArrangedSubview($0); $0.removeFromSuperview() }
        list.frame = NSRect(x: 0, y: 0, width: 380, height: max(300, entries.count * 88))
        for entry in entries {
            do {
                let image = try store.image(for: entry.id)
                guard let source = CGImageSourceCreateWithData(image.pngData as CFData, nil),
                      let thumbnail = CGImageSourceCreateThumbnailAtIndex(source, 0, [kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: 240] as CFDictionary) else { throw CocoaError(.fileReadCorruptFile) }
                let row = NSStackView(); row.orientation = .horizontal; row.spacing = 12
                let preview = NSButton(image: NSImage(cgImage: thumbnail, size: NSSize(width: 200, height: 80)), target: self, action: #selector(selectAction(_:)))
                preview.imageScaling = .scaleProportionallyUpOrDown; preview.identifier = NSUserInterfaceItemIdentifier(entry.id.uuidString); preview.setAccessibilityLabel("选择签名 \(entry.createdAt.formatted())")
                preview.widthAnchor.constraint(equalToConstant: 200).isActive = true; preview.heightAnchor.constraint(equalToConstant: 80).isActive = true
                let remove = NSButton(title: "删除", target: self, action: #selector(deleteAction(_:))); remove.identifier = preview.identifier; remove.setAccessibilityLabel("删除签名 \(entry.createdAt.formatted())")
                row.addArrangedSubview(preview); row.addArrangedSubview(remove); list.addArrangedSubview(row)
            } catch { presentError(error.localizedDescription); break }
        }
    }
    @objc private func importAction() { importPNG() }
    @objc private func drawAction() { panel?.close(); onDraw() }
    @objc private func selectAction(_ sender: NSButton) { if let id = sender.identifier.flatMap({ UUID(uuidString: $0.rawValue) }) { select(id: id) } }
    @objc private func deleteAction(_ sender: NSButton) { if let id = sender.identifier.flatMap({ UUID(uuidString: $0.rawValue) }) { delete(id: id) } }
}
