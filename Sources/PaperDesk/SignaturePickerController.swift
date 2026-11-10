import AppKit
import ImageIO
import PaperCore

/// Document-owned inspector, never a floating library window.
@MainActor final class SignaturePickerController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    let store: SignatureAssetStore
    let tableView = SignatureHistoryTable()
    private let onSelectImage: (SignatureImage) -> Void
    private let presentError: @MainActor (String) -> Void
    private let empty = NSTextField(wrappingLabelWithString: "点击上方 + 添加签名")
    private(set) var editor: SignatureEditorController?
    // Row identities belong to the displayed snapshot, not a mutable store
    // shared by other document windows. Reentry explicitly refreshes history.
    private(set) var entries: [SignatureAsset] = []
    var selectedID: UUID? { entries.indices.contains(tableView.selectedRow) ? entries[tableView.selectedRow].id : nil }

    init(store: SignatureAssetStore, onSelectImage: @escaping (SignatureImage) -> Void,
         presentError: @escaping @MainActor (String) -> Void = { message in
             let alert = NSAlert(); alert.messageText = "签名库操作失败"; alert.informativeText = message; alert.runModal()
         }) {
        self.store = store; self.onSelectImage = onSelectImage; self.presentError = presentError
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 240, height: 480))
        view.wantsLayer = true; view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        view.setAccessibilityLabel("签名检查器")
        let plus = SignatureAddButton(image: NSImage(systemSymbolName: "plus", accessibilityDescription: "添加签名")!, target: self, action: #selector(addAction))
        plus.isBordered = false; plus.setAccessibilityLabel("添加签名"); plus.toolTip = "添加签名"
        let separator = SignatureSeparator(); separator.boxType = .separator
        let scroll = NSScrollView(); scroll.hasVerticalScroller = true; scroll.hasHorizontalScroller = false
        scroll.drawsBackground = false; scroll.autohidesScrollers = true
        tableView.headerView = nil; tableView.rowHeight = 84; tableView.intercellSpacing = NSSize(width: 0, height: 8)
        tableView.allowsMultipleSelection = false; tableView.allowsEmptySelection = false
        tableView.backgroundColor = .clear; tableView.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
        tableView.setAccessibilityLabel("签名列表")
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("signature")); column.width = 224
        column.resizingMask = .autoresizingMask; tableView.addTableColumn(column)
        tableView.dataSource = self; tableView.delegate = self
        tableView.target = self; tableView.action = #selector(selectAction)
        tableView.deleteSelection = { [weak self] in self?.deleteSelected() }
        tableView.menuForRow = { [weak self] row in self?.contextMenu(forRow: row) }
        scroll.documentView = tableView
        empty.alignment = .center; empty.textColor = .secondaryLabelColor
        [plus, separator, scroll, empty].forEach { $0.translatesAutoresizingMaskIntoConstraints = false; view.addSubview($0) }
        NSLayoutConstraint.activate([
            plus.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 6), plus.topAnchor.constraint(equalTo: view.topAnchor, constant: 10),
            plus.widthAnchor.constraint(equalToConstant: 24), plus.heightAnchor.constraint(equalToConstant: 24),
            separator.topAnchor.constraint(equalTo: plus.bottomAnchor, constant: 8), separator.leadingAnchor.constraint(equalTo: view.leadingAnchor), separator.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: separator.bottomAnchor), scroll.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 8), scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -8),
            empty.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 20), empty.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -20), empty.centerYAnchor.constraint(equalTo: view.centerYAnchor)
        ])
    }
    func reloadHistory() {
        _ = view
        let selection = selectedID
        do { try store.reload(); render(selecting: selection) } catch { presentError(error.localizedDescription) }
    }
    private func render(selecting id: UUID?) {
        entries = store.entries
        tableView.reloadData(); empty.isHidden = !entries.isEmpty
        if let id, let row = entries.firstIndex(where: { $0.id == id }) { tableView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false) }
        else if !entries.isEmpty { tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false) }
    }
    func select(id: UUID) {
        guard let row = entries.firstIndex(where: { $0.id == id }) else { return }
        tableView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        do { onSelectImage(try store.image(for: id)) } catch { presentError(error.localizedDescription) }
    }
    func delete(id: UUID) {
        let row = entries.firstIndex(where: { $0.id == id }) ?? 0
        let oldSelection = selectedID
        do {
            try store.remove(id: id)
            let next = oldSelection == id ? (entries.isEmpty ? nil : entries[min(row, entries.count - 1)].id) : oldSelection
            render(selecting: next)
        } catch { presentError(error.localizedDescription) }
    }
    func contextMenu(forRow row: Int) -> NSMenu? {
        guard entries.indices.contains(row) else { return nil }
        // Bind the menu item to the clicked asset, not the previous selection.
        tableView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        let menu = NSMenu()
        let item = NSMenuItem(title: "删除", action: #selector(deleteContext(_:)), keyEquivalent: "")
        item.target = self; item.representedObject = entries[row].id; item.setAccessibilityLabel("删除签名")
        menu.addItem(item); return menu
    }
    func dismissEditor() { editor?.cancel(); editor = nil }
    @objc private func addAction() {
        guard let window = view.window, window.attachedSheet == nil else { return }
        let editor = SignatureEditorController(store: store, onCreate: { [weak self] asset in
            self?.reloadHistory(); self?.select(id: asset.id)
        }, presentError: presentError)
        self.editor = editor; editor.present(from: window)
    }
    @objc private func selectAction() { if let id = selectedID { select(id: id) } }
    private func deleteSelected() { if let id = selectedID { delete(id: id) } }
    @objc private func deleteContext(_ item: NSMenuItem) { if let id = item.representedObject as? UUID { delete(id: id) } }
    func numberOfRows(in tableView: NSTableView) -> Int { entries.count }
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard entries.indices.contains(row) else { return nil }
        let entry = entries[row]
        let cell = NSTableCellView(frame: NSRect(x: 0, y: 0, width: 224, height: 84))
        let preview = NSImageView(frame: cell.bounds.insetBy(dx: 8, dy: 8)); preview.autoresizingMask = [.width, .height]
        preview.imageScaling = .scaleProportionallyUpOrDown
        cell.imageView = preview; cell.addSubview(preview)
        do {
            let image = try store.image(for: entry.id)
            guard let source = CGImageSourceCreateWithData(image.pngData as CFData, nil),
                  let thumbnail = CGImageSourceCreateThumbnailAtIndex(source, 0, [kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: 240] as CFDictionary) else { throw CocoaError(.fileReadCorruptFile) }
            preview.image = NSImage(cgImage: thumbnail, size: NSSize(width: thumbnail.width, height: thumbnail.height))
            cell.setAccessibilityLabel("选择签名 \(entry.createdAt.formatted())")
        } catch {
            let text = NSTextField(labelWithString: "签名不可用")
            text.frame = NSRect(x: 8, y: 32, width: 208, height: 20); text.autoresizingMask = [.width]
            text.alignment = .center; text.textColor = .secondaryLabelColor; cell.textField = text; cell.addSubview(text)
            cell.setAccessibilityLabel("签名不可用，可按删除键删除")
        }
        return cell
    }
}

private final class SignatureAddButton: NSButton {
    override var alignmentRectInsets: NSEdgeInsets { NSEdgeInsetsZero }
}
private final class SignatureSeparator: NSBox {
    override var alignmentRectInsets: NSEdgeInsets { NSEdgeInsetsZero }
}

@MainActor final class SignatureHistoryTable: NSTableView {
    var deleteSelection: (() -> Void)?
    var menuForRow: ((Int) -> NSMenu?)?
    override func keyDown(with event: NSEvent) {
        if event.keyCode == 51 || event.keyCode == 117 { deleteSelection?() }
        else { super.keyDown(with: event) }
    }
    override func menu(for event: NSEvent) -> NSMenu? { menuForRow?(row(at: convert(event.locationInWindow, from: nil))) }
}
