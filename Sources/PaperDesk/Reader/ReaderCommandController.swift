import AppKit
import PDFKit

@MainActor public final class ReaderCommandController: NSObject, NSMenuItemValidation {
    public let session: ReaderSession
    public let sidebar: ReaderSidebarController
    public var externalCommand: ((ReaderCommand) -> Bool)?
    public var interceptCommand: ((ReaderCommand) -> Bool?)?
    public var canExternalCommand: ((ReaderCommand) -> Bool)?
    public var setSidebarVisible: ((Bool) -> Void)?
    public var onError: ((Error) -> Void)?
    private let zoomMenus = NSHashTable<NSMenu>.weakObjects()
    public private(set) var sidebarVisible = true
    public var continuous: Bool { [.singlePageContinuous, .twoUpContinuous].contains(session.pdfView.displayMode) }
    public var layout: ReaderLayout { session.currentLayout }
    public init(session: ReaderSession, sidebar: ReaderSidebarController) { self.session = session; self.sidebar = sidebar }
    public func makeMenu(commands: [ReaderCommand], title: String) -> NSMenu {
        let menu = NSMenu(title: title)
        for command in commands {
            let shortcut = ReaderShortcut.forCommand(command)
            let item = NSMenuItem(title: command.title, action: #selector(runItem(_:)), keyEquivalent: shortcut.key)
            item.keyEquivalentModifierMask = shortcut.modifiers; item.target = self
            item.representedObject = command.rawValue
            menu.addItem(item)
        }
        return menu
    }
    public func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        // NSPopUpButton items (including the current arbitrary percentage added
        // by the toolbar) are not ReaderCommand items. AppKit still asks this
        // target to validate them when opening the popup's auto-enabling menu.
        if let menu = menuItem.menu, zoomMenus.contains(menu) {
            guard canRun(.actualSize) else { return false }
            return menuItem.title == "适合页面" || menuItem.title == "适合宽度" || (try? ZoomInput.parse(menuItem.title)) != nil
        }
        guard let raw = menuItem.representedObject as? String, let command = ReaderCommand(rawValue: raw) else { return false }
        switch command {
        case .singlePage: menuItem.state = layout == .single ? .on : .off
        case .twoPages: menuItem.state = layout == .twoPages ? .on : .off
        case .cover: menuItem.state = layout == .twoPagesWithCover ? .on : .off
        default: break
        }
        return canRun(command)
    }
    private func canRun(_ command: ReaderCommand) -> Bool {
        switch command {
        case .newTab, .nextTab, .previousTab, .closeOtherTabs: return externalCommand != nil && (canExternalCommand?(command) ?? true)
        default: break
        }
        guard let document = session.pdfView.document, !document.isLocked, document.pageCount > 0 else { return false }
        switch command {
        case .find: return session.allowsCopying
        case .findSelection: return session.allowsCopying && !(session.pdfView.currentSelection?.string?.isEmpty ?? true)
        case .findNext, .findPrevious: return session.allowsCopying && !session.hits.isEmpty
        default: return true
        }
    }
    @discardableResult public func run(_ command: ReaderCommand) -> Bool {
        if let handled = interceptCommand?(command) { return handled }
        guard canRun(command) else { return false }
        do {
            switch command {
            case .find:
                reveal(.search); sidebar.view.window?.makeFirstResponder(sidebar.searchField)
            case .findNext: session.nextHit()
            case .findPrevious: session.nextHit(backward: true)
            case .findSelection:
                guard let selected = session.pdfView.currentSelection?.string else { return false }
                reveal(.search); sidebar.searchField.stringValue = selected; try session.search(selected)
            case .singlePage: session.setLayout(.single, continuous: continuous)
            case .twoPages: session.setLayout(.twoPages, continuous: continuous)
            case .cover: session.setLayout(.twoPagesWithCover, continuous: continuous)
            case .fitPage: try session.setZoom(.fitPage)
            case .fitWidth: try session.setZoom(.fitWidth)
            case .actualSize: try session.setZoom(.percent(100))
            case .zoomIn: try session.setZoom(.percent(min(6400, session.pdfView.scaleFactor * 125)))
            case .zoomOut: try session.setZoom(.percent(max(10, session.pdfView.scaleFactor * 80)))
            case .outline: reveal(.outline)
            case .thumbnails: reveal(.thumbnails)
            case .toggleSidebar:
                sidebarVisible.toggle(); sidebar.view.isHidden = !sidebarVisible; setSidebarVisible?(sidebarVisible)
            case .newTab, .nextTab, .previousTab, .closeOtherTabs:
                return externalCommand?(command) ?? false
            }
            sidebar.refresh()
            return true
        } catch { onError?(error); return false }
    }
    private func reveal(_ mode: ReaderSidebarMode) {
        sidebarVisible = true; sidebar.view.isHidden = false
        sidebar.show(mode); setSidebarVisible?(true)
    }
    public func setContinuous(_ value: Bool) { session.setLayout(layout, continuous: value) }
    public func makeZoomPopup() -> NSPopUpButton {
        let popup = NSPopUpButton(frame: .zero, pullsDown: false)
        popup.addItems(withTitles: ["适合页面", "适合宽度", "25%", "50%", "75%", "100%", "125%", "150%", "200%", "400%"])
        popup.target = self; popup.action = #selector(selectZoom(_:)); popup.selectItem(withTitle: "100%")
        if let menu = popup.menu { zoomMenus.add(menu) }
        popup.setAccessibilityLabel("阅读器缩放")
        return popup
    }
    @objc private func runItem(_ item: NSMenuItem) {
        guard let raw = item.representedObject as? String, let command = ReaderCommand(rawValue: raw) else { return }
        _ = run(command)
    }
    @objc private func selectZoom(_ popup: NSPopUpButton) {
        guard canRun(.actualSize), let title = popup.titleOfSelectedItem else { return }
        do {
            let zoom: ReaderZoom = title == "适合页面" ? .fitPage : (title == "适合宽度" ? .fitWidth : try ZoomInput.parse(title))
            try session.setZoom(zoom)
        } catch { onError?(error) }
    }
}

private extension ReaderCommand {
    var title: String {
        switch self {
        case .find: return "查找…"
        case .findNext: return "查找下一个"
        case .findPrevious: return "查找上一个"
        case .findSelection: return "使用选中文本查找"
        case .singlePage: return "单页"
        case .twoPages: return "双页"
        case .cover: return "双页，首页单独显示"
        case .fitPage: return "适合页面"
        case .fitWidth: return "适合宽度"
        case .actualSize: return "实际大小 (100%)"
        case .zoomIn: return "放大"
        case .zoomOut: return "缩小"
        case .outline: return "大纲"
        case .thumbnails: return "缩略图侧栏"
        case .toggleSidebar: return "显示/隐藏侧栏"
        case .newTab: return "新标签"
        case .nextTab: return "下一个标签"
        case .previousTab: return "上一个标签"
        case .closeOtherTabs: return "关闭其它标签"
        }
    }
}
