import AppKit

@MainActor enum ToolIcon {
    static func load(id: String, directory: URL?) -> NSImage? {
        guard ToolCatalog.descriptor(id: id) != nil, let directory,
              let image = NSImage(contentsOf: directory.appendingPathComponent(id + ".png")) else { return nil }
        image.isTemplate = false
        return image
    }
}

enum ToolIconSkin: String, CaseIterable {
    case classic
    case artMinimal = "art-minimal"
    var title: String { self == .classic ? "经典线框" : "极简艺术" }
    var detail: String { self == .classic ? "轻盈彩色线条" : "蒙德里安几何 · 梵高蓝黄" }
}

@MainActor final class ToolIconSkinStore {
    static let preferenceKey = "PDFMaster.toolIconSkin"
    static let didChange = Notification.Name("PDFMaster.toolIconSkinDidChange")
    private let defaults: UserDefaults
    private let ids: Set<String>
    private var images: [ToolIconSkin: [String: NSImage]] = [:]
    init(defaults: UserDefaults, resourceRoot: URL?, toolIDs: [String]) {
        self.defaults = defaults
        ids = Set(toolIDs.filter { !$0.isEmpty && $0.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-") } })
        guard let resourceRoot else { return }
        for skin in ToolIconSkin.allCases {
            let directory = resourceRoot.appendingPathComponent(skin == .classic ? "ToolIcons" : "ToolSkins/art-minimal")
            var entries: [String: NSImage] = [:]
            for id in ids {
                let url = directory.appendingPathComponent(id + ".png")
                guard let data = try? Data(contentsOf: url),
                      let bitmap = NSBitmapImageRep(data: data), bitmap.pixelsWide > 0,
                      let image = NSImage(data: data) else { continue }
                image.isTemplate = false; entries[id] = image
            }
            images[skin] = entries
        }
    }
    var selected: ToolIconSkin {
        let requested = defaults.string(forKey: Self.preferenceKey).flatMap(ToolIconSkin.init(rawValue:)) ?? .artMinimal
        if isAvailable(requested) { return requested }
        return ToolIconSkin.allCases.first(where: isAvailable) ?? .classic
    }
    func isAvailable(_ skin: ToolIconSkin) -> Bool { !ids.isEmpty && images[skin]?.count == ids.count }
    func select(_ skin: ToolIconSkin) throws {
        guard isAvailable(skin) else { throw SkinError.incomplete }
        let old = selected
        defaults.set(skin.rawValue, forKey: Self.preferenceKey)
        if old != skin { NotificationCenter.default.post(name: Self.didChange, object: nil) }
    }
    func image(id: String, skin: ToolIconSkin? = nil) -> NSImage? {
        guard ids.contains(id) else { return nil }
        let resolved = skin ?? selected
        guard isAvailable(resolved) else { return nil }
        return images[resolved]?[id]
    }
    enum SkinError: LocalizedError {
        case incomplete
        var errorDescription: String? { "此皮肤图标不完整，已保留当前皮肤。请重新安装完整资源包。" }
    }
}

@MainActor final class ToolIconSkinButton: NSButton {
    let store: ToolIconSkinStore
    private let popover = NSPopover()
    init(store: ToolIconSkinStore) {
        self.store = store
        super.init(frame: .zero)
        title = "皮肤"; bezelStyle = .rounded
        image = NSImage(systemSymbolName: "paintpalette", accessibilityDescription: nil)
        imagePosition = .imageLeading; font = .systemFont(ofSize: 13)
        setAccessibilityLabel("切换整套工具图标皮肤")
        toolTip = "当前：\(store.isAvailable(store.selected) ? store.selected.title : "系统图标（皮肤资源不完整）") · 只改变工具图标"
        target = self; action = #selector(showPicker)
        widthAnchor.constraint(equalToConstant: 82).isActive = true
        heightAnchor.constraint(equalToConstant: 32).isActive = true
    }
    required init?(coder: NSCoder) { fatalError() }
    @objc private func showPicker() {
        let controller = NSViewController(); controller.view = makePickerContent()
        popover.contentViewController = controller; popover.behavior = .transient
        popover.show(relativeTo: bounds, of: self, preferredEdge: .minY)
    }
    func makePickerContent() -> NSView {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 344, height: 292))
        root.wantsLayer = true; root.layer?.backgroundColor = NSColor(calibratedWhite: 0.10, alpha: 1).cgColor
        let heading = NSTextField(labelWithString: "图标皮肤")
        heading.font = .systemFont(ofSize: 18, weight: .semibold); heading.textColor = .white
        heading.frame = NSRect(x: 20, y: 251, width: 300, height: 26); root.addSubview(heading)
        for (index, skin) in [ToolIconSkin.artMinimal, .classic].enumerated() {
            let row = NSView(frame: NSRect(x: 16, y: 140 - index * 110, width: 312, height: 104))
            row.wantsLayer = true; row.layer?.cornerRadius = 12; row.layer?.backgroundColor = NSColor(calibratedWhite: 0.15, alpha: 1).cgColor
            let current = store.selected == skin && store.isAvailable(skin)
            row.layer?.borderWidth = current ? 1.5 : 0; row.layer?.borderColor = NSColor.systemBlue.cgColor
            for (imageIndex, id) in ["annotate", "pages", "password", "image-to-pdf"].enumerated() {
                let preview = NSImageView(frame: NSRect(x: 14 + imageIndex * 38, y: 12, width: 30, height: 30))
                preview.image = store.image(id: id, skin: skin); preview.imageScaling = .scaleProportionallyUpOrDown
                preview.contentTintColor = nil; preview.setAccessibilityElement(false); row.addSubview(preview)
            }
            let name = NSTextField(labelWithString: skin.title)
            name.font = .systemFont(ofSize: 14, weight: .semibold); name.textColor = .white
            name.frame = NSRect(x: 14, y: 74, width: 190, height: 20); row.addSubview(name)
            let subtitle = NSTextField(labelWithString: store.isAvailable(skin) ? skin.detail : "资源不完整，暂不可选")
            subtitle.font = .systemFont(ofSize: 11); subtitle.textColor = .lightGray
            subtitle.frame = NSRect(x: 14, y: 53, width: 280, height: 17); row.addSubview(subtitle)
            let select = NSButton(title: current ? "已选用" : "选用", target: self, action: #selector(choose(_:)))
            select.identifier = NSUserInterfaceItemIdentifier(skin.rawValue); select.bezelStyle = .rounded
            select.state = current ? .on : .off; select.isEnabled = store.isAvailable(skin)
            select.setAccessibilityLabel("选用\(skin.title)整套图标")
            select.frame = NSRect(x: 229, y: 67, width: 70, height: 28); row.addSubview(select)
            root.addSubview(row)
        }
        let note = NSTextField(labelWithString: "即时生效 · 自动记住选择 · 不更改文档")
        note.font = .systemFont(ofSize: 10); note.textColor = .lightGray
        note.frame = NSRect(x: 20, y: 7, width: 306, height: 17); root.addSubview(note)
        return root
    }
    @objc private func choose(_ sender: NSButton) {
        guard let value = sender.identifier?.rawValue, let skin = ToolIconSkin(rawValue: value) else { return }
        do { try store.select(skin); toolTip = "当前：\(store.selected.title)"; popover.performClose(nil) }
        catch { let alert = NSAlert(); alert.messageText = "无法切换皮肤"; alert.informativeText = error.localizedDescription; alert.runModal() }
    }
}
