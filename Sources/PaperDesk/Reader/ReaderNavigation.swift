import PDFKit
import AppKit

public struct ReaderOutlineItem {
    public let title: String
    public let depth: Int
    public let path: [Int]
    public let destination: PDFDestination?
    public let pageIndex: Int?
    public static func project(_ document: PDFDocument) -> [ReaderOutlineItem] {
        guard let root = document.outlineRoot else { return [] }
        var result: [ReaderOutlineItem] = []
        var visited = Set<ObjectIdentifier>()
        func children(_ parent: PDFOutline, path: [Int]) {
            guard path.count < 128, visited.insert(ObjectIdentifier(parent)).inserted else { return }
            for index in 0..<parent.numberOfChildren {
                guard let child = parent.child(at: index) else { continue }
                let destination = child.destination ?? (child.action as? PDFActionGoTo)?.destination
                let pageIndex = destination?.page.map { document.index(for: $0) }
                let localIndex = pageIndex.flatMap { $0 == NSNotFound ? nil : $0 }
                let childPath = path + [index]
                result.append(.init(title: child.label ?? "未命名", depth: path.count, path: childPath,
                                    destination: localIndex == nil ? nil : destination, pageIndex: localIndex))
                children(child, path: childPath)
            }
        }
        children(root, path: [])
        return result
    }
}

/// Plain arrow keys intentionally page by one sheet when the PDF view owns focus.
/// Text controls never call this path because AppKit delivers their key events to
/// the field editor instead. Annotation editing gets first refusal in InkPDFView.
enum ReaderArrowNavigation {
    static func pageDelta(for keyCode: UInt16) -> Int? {
        switch keyCode {
        case 123, 126: return -1 // left / up
        case 124, 125: return 1  // right / down
        default: return nil
        }
    }

    static func destination(keyCode: UInt16, current: Int, pageCount: Int) -> Int? {
        guard let delta = pageDelta(for: keyCode), pageCount > 0,
              (0..<pageCount).contains(current) else { return nil }
        return min(max(current + delta, 0), pageCount - 1)
    }
}

public enum ReaderSidebarMode: Int, CaseIterable { case thumbnails, outline, search, annotations }

public enum ReaderAnnotationSort { case page, newestFirst }

/// Delete belongs to this list only while it owns keyboard focus.
public final class ReaderAnnotationTableView: NSTableView {
    var onDeleteSelection: (() -> Bool)?
    public override func keyDown(with event: NSEvent) {
        if PageSelectionDeleteKey.matches(event, in: self), onDeleteSelection?() == true { return }
        super.keyDown(with: event)
    }
}

public struct ReaderAnnotationItem {
    public let pageIndex: Int
    public let annotationIndex: Int
    public let type: String
    public let color: NSColor
    public let contents: String
    public let excerpt: String
    public let modificationDate: Date?
    fileprivate let documentID: ObjectIdentifier
    fileprivate let pageID: ObjectIdentifier
    fileprivate let annotationID: ObjectIdentifier
    // Retain the projected object until the next refresh so an allocator cannot
    // reuse a removed annotation's address and accidentally validate a stale row.
    fileprivate let annotation: PDFAnnotation
    public var typeTitle: String {
        switch type {
        case "Highlight": return "高亮"
        case "Underline": return "下划线"
        case "StrikeOut": return "删除线"
        case "Ink": return "手绘"
        case "FreeText": return "文字"
        case "Text": return "便笺"
        case "Square": return "矩形"
        case "Circle": return "椭圆"
        case "Line": return "直线 / 箭头"
        default: return type
        }
    }
}

@MainActor public final class ReaderSidebarController: NSViewController, NSTableViewDataSource, NSTableViewDelegate, NSSearchFieldDelegate {
    public let session: ReaderSession
    public private(set) var mode: ReaderSidebarMode = .thumbnails
    public let thumbnailView = ReaderPageThumbnailView()
    public let table = ReaderAnnotationTableView()
    public let searchField = NSSearchField()
    public private(set) var outlineRows: [ReaderOutlineItem] = []
    public private(set) var annotationRows: [ReaderAnnotationItem] = []
    public private(set) var selectedAnnotationRow: Int?
    private(set) var annotationToolButtons: [ToolStripButton] = []
    public var onAnnotationSelection: ((Int, Int) -> Void)?
    public var onDeleteAnnotation: ((Int, Int) -> Void)? { didSet { updateAnnotationDeletion() } }
    public var canDeleteAnnotation: ((Int, Int) -> Bool)? { didSet { updateAnnotationDeletion() } }
    public var onDeletePages: ((IndexSet) -> Void)?
    public var onAnnotationTool: ((String) -> Void)? { didSet { updateAnnotationTools() } }
    public let annotationEmptyHint = "使用批注和绘画工具标记PDF"
    public var isAnnotationEmptyStateVisible: Bool { mode == .annotations && annotationRows.isEmpty }
    public var annotationSummary: String {
        guard session.allowsCopying else { return "" }
        return annotationRows.map { item in
            let detail = [item.contents, item.excerpt].filter { !$0.isEmpty }.joined(separator: "\n")
            return "第 \(item.pageIndex + 1) 页 · \(item.typeTitle)" + (detail.isEmpty ? "" : "\n" + detail)
        }.joined(separator: "\n\n")
    }
    public private(set) var annotationSort: ReaderAnnotationSort = .page
    public func setAnnotationSort(_ sort: ReaderAnnotationSort) { annotationSort = sort; refresh() }
    private func validatedAnnotationRow(_ row: Int) -> ReaderAnnotationItem? {
        guard annotationRows.indices.contains(row), let document = session.pdfView.document else { return nil }
        let item = annotationRows[row]
        guard ObjectIdentifier(document) == item.documentID,
              let page = document.page(at: item.pageIndex), ObjectIdentifier(page) == item.pageID,
              page.annotations.indices.contains(item.annotationIndex),
              page.annotations[item.annotationIndex] === item.annotation else { return nil }
        return item
    }
    @discardableResult public func navigateToAnnotationRow(_ row: Int) -> Bool {
        guard let item = validatedAnnotationRow(row), let page = session.pdfView.document?.page(at: item.pageIndex) else { return false }
        selectedAnnotationRow = row
        syncSelection = true
        table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        syncSelection = false
        session.pdfView.go(to: page)
        let bounds = item.annotation.bounds
        if bounds.origin.x.isFinite, bounds.origin.y.isFinite, bounds.width.isFinite, bounds.height.isFinite,
           bounds.width > 0, bounds.height > 0 {
            session.pdfView.go(to: bounds.insetBy(dx: -12, dy: -12), on: page)
        }
        onAnnotationSelection?(item.pageIndex, item.annotationIndex)
        updateAnnotationDeletion()
        return true
    }
    private var deletableAnnotation: ReaderAnnotationItem? {
        guard mode == .annotations, onDeleteAnnotation != nil,
              let row = selectedAnnotationRow, let item = validatedAnnotationRow(row),
              canDeleteAnnotation?(item.pageIndex, item.annotationIndex) ?? true else { return nil }
        return item
    }
    private func updateAnnotationDeletion() { annotationDeleteButton.isEnabled = deletableAnnotation != nil }
    @discardableResult private func deleteSelectedAnnotationRow() -> Bool {
        guard let item = deletableAnnotation else { updateAnnotationDeletion(); return false }
        onDeleteAnnotation?(item.pageIndex, item.annotationIndex)
        refresh()
        return true
    }
    @objc private func deleteAnnotationFromButton(_ sender: NSButton) { deleteSelectedAnnotationRow() }
    private let status = NSTextField(wrappingLabelWithString: "")
    private let scroll = NSScrollView()
    private let modes = NSSegmentedControl(labels: ["缩略图", "大纲", "搜索", "A"], trackingMode: .selectOne, target: nil, action: nil)
    private var syncSelection = false
    private let searchNavigation = NSStackView()
    private let annotationHeader = NSStackView()
    private let annotationEmptyView = NSView()
    private let annotationOptions = NSButton()
    private let annotationDeleteButton = NSButton()
    public init(session: ReaderSession) { self.session = session; super.init(nibName: nil, bundle: nil) }
    required init?(coder: NSCoder) { fatalError("init(coder:) unsupported") }
    public override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 176, height: 700))
        view.appearance = NSAppearance(named: .darkAqua)
        view.wantsLayer = true; view.layer?.backgroundColor = NSColor(calibratedWhite: 0.125, alpha: 1).cgColor
        let stack = NSStackView(); stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(stack)
        NSLayoutConstraint.activate([stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 8), stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -8), stack.topAnchor.constraint(equalTo: view.topAnchor, constant: 10), stack.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -8)])
        modes.target = self; modes.action = #selector(changeSidebarMode); modes.selectedSegment = 0
        modes.segmentStyle = .texturedRounded
        for (index, symbol, title) in [(0, "square.grid.2x2", "缩略图"), (1, "list.bullet", "大纲"), (2, "magnifyingglass", "搜索"), (3, "a", "批注")] {
            modes.setLabel(index == 3 ? "A" : "", forSegment: index)
            if index != 3 { modes.setImage(NSImage(systemSymbolName: symbol, accessibilityDescription: title), forSegment: index) }
            modes.setToolTip(title, forSegment: index)
            modes.setWidth(34, forSegment: index)
        }
        modes.setContentHuggingPriority(.defaultLow, for: .horizontal)
        modes.setAccessibilityLabel("阅读器侧栏模式")
        searchField.placeholderString = "搜索文档文字"; searchField.delegate = self
        searchField.setAccessibilityLabel("搜索文档文字")
        searchNavigation.orientation = .horizontal
        for (label, action) in [("上一处", #selector(previousHit)), ("下一处", #selector(nextHit)), ("取消", #selector(cancel))] {
            let button = NSButton(title: label, target: self, action: action); button.bezelStyle = .inline
            searchNavigation.addArrangedSubview(button)
        }
        status.textColor = .secondaryLabelColor; status.font = .systemFont(ofSize: 11)
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("result")); table.addTableColumn(column)
        table.headerView = nil; table.delegate = self; table.dataSource = self; table.rowHeight = 54
        table.backgroundColor = .clear; table.setAccessibilityLabel("大纲或搜索结果列表")
        table.onDeleteSelection = { [weak self] in self?.deleteSelectedAnnotationRow() ?? false }
        table.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        column.minWidth = 40
        scroll.documentView = table; scroll.hasVerticalScroller = true; scroll.drawsBackground = false
        thumbnailView.pdfView = session.pdfView; thumbnailView.thumbnailSize = .init(width: 124, height: 164)
        thumbnailView.onDeleteSelection = { [weak self] in
            guard let self, self.mode == .thumbnails,
                  let document = self.session.pdfView.document else { return }
            let selected = self.thumbnailView.selectedPages ?? self.session.pdfView.currentPage.map { [$0] } ?? []
            let indexes = IndexSet(selected.compactMap { page -> Int? in
                let index = document.index(for: page)
                return index == NSNotFound ? nil : index
            })
            guard !indexes.isEmpty else { return }
            self.onDeletePages?(indexes)
        }
        thumbnailView.backgroundColor = NSColor(calibratedWhite: 0.125, alpha: 1)
        configureAnnotationViews()
        for child in [modes, annotationHeader, searchField, searchNavigation, status, annotationEmptyView, scroll, thumbnailView] {
            stack.addArrangedSubview(child)
            child.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        }
        scroll.heightAnchor.constraint(greaterThanOrEqualToConstant: 80).isActive = true
        thumbnailView.heightAnchor.constraint(greaterThanOrEqualToConstant: 80).isActive = true
        refresh()
    }
    private func configureAnnotationViews() {
        annotationHeader.orientation = .horizontal; annotationHeader.alignment = .centerY
        let title = NSTextField(labelWithString: "批注")
        title.font = .systemFont(ofSize: 13, weight: .medium)
        title.setContentHuggingPriority(.defaultLow, for: .horizontal)
        annotationOptions.image = NSImage(systemSymbolName: "ellipsis", accessibilityDescription: "批注选项")
        annotationOptions.isBordered = false; annotationOptions.target = self
        annotationOptions.action = #selector(showAnnotationOptions(_:))
        annotationOptions.toolTip = "批注排序与复制摘要"
        annotationOptions.setAccessibilityLabel("批注选项")
        annotationDeleteButton.image = NSImage(systemSymbolName: "trash", accessibilityDescription: "删除所选批注")
        annotationDeleteButton.isBordered = false; annotationDeleteButton.target = self
        annotationDeleteButton.action = #selector(deleteAnnotationFromButton(_:))
        annotationDeleteButton.toolTip = "删除所选批注（Delete），可撤销"
        annotationDeleteButton.setAccessibilityLabel("删除所选批注")
        annotationHeader.addArrangedSubview(title); annotationHeader.addArrangedSubview(annotationDeleteButton); annotationHeader.addArrangedSubview(annotationOptions)
        annotationDeleteButton.widthAnchor.constraint(equalToConstant: 24).isActive = true
        annotationOptions.widthAnchor.constraint(equalToConstant: 24).isActive = true
        annotationHeader.heightAnchor.constraint(equalToConstant: 24).isActive = true
        let content = NSStackView()
        content.orientation = .vertical; content.alignment = .centerX; content.spacing = 14
        content.translatesAutoresizingMaskIntoConstraints = false
        annotationEmptyView.addSubview(content)
        let glyph = AnnotationEmptyGlyphView()
        glyph.widthAnchor.constraint(equalToConstant: 98).isActive = true
        glyph.heightAnchor.constraint(equalToConstant: 98).isActive = true
        let hint = NSTextField(wrappingLabelWithString: annotationEmptyHint)
        hint.alignment = .center; hint.font = .systemFont(ofSize: 12)
        hint.textColor = .secondaryLabelColor; hint.maximumNumberOfLines = 3
        let tools = NSStackView(); tools.orientation = .horizontal; tools.spacing = 4
        let availableTools = ReferenceToolStrip.tools(for: .annotation)
        for id in ["highlight", "pen", "text", "note"] {
            guard let tool = availableTools.first(where: { $0.id == id }) else { continue }
            let button = ToolStripButton(tool: tool, target: self, action: #selector(activateAnnotationTool(_:)))
            button.setAccessibilityLabel("批注工具 · \(tool.title)")
            annotationToolButtons.append(button); tools.addArrangedSubview(button)
        }
        for child in [glyph, hint, tools] { content.addArrangedSubview(child) }
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: annotationEmptyView.leadingAnchor, constant: 8),
            content.trailingAnchor.constraint(equalTo: annotationEmptyView.trailingAnchor, constant: -8),
            content.centerYAnchor.constraint(equalTo: annotationEmptyView.centerYAnchor),
            hint.widthAnchor.constraint(equalTo: content.widthAnchor),
            annotationEmptyView.heightAnchor.constraint(greaterThanOrEqualToConstant: 220)
        ])
    }
    private func updateAnnotationTools() {
        for button in annotationToolButtons {
            button.isEnabled = onAnnotationTool != nil && session.pdfView.document != nil
        }
    }
    private func refreshAnnotations() {
        let selected = selectedAnnotationRow.flatMap { annotationRows.indices.contains($0) ? annotationRows[$0] : nil }
        var rows: [ReaderAnnotationItem] = []
        if let document = session.pdfView.document {
            for pageIndex in 0..<document.pageCount {
                guard let page = document.page(at: pageIndex) else { continue }
                for (index, annotation) in page.annotations.enumerated() {
                    guard let type = annotation.type, !["Widget", "Link", "Popup"].contains(type) else { continue }
                    let excerpt: String
                    if session.allowsCopying && ["Highlight", "Underline", "StrikeOut"].contains(type) {
                        excerpt = String((page.selection(for: annotation.bounds)?.string ?? "")
                            .split(whereSeparator: \.isWhitespace).joined(separator: " ").prefix(240))
                    } else { excerpt = "" }
                    rows.append(ReaderAnnotationItem(pageIndex: pageIndex, annotationIndex: index, type: type,
                        color: annotation.color, contents: annotation.contents ?? "", excerpt: excerpt,
                        modificationDate: annotation.modificationDate, documentID: ObjectIdentifier(document),
                        pageID: ObjectIdentifier(page), annotationID: ObjectIdentifier(annotation), annotation: annotation))
                }
            }
        }
        if annotationSort == .newestFirst {
            rows.sort {
                let a = $0.modificationDate ?? .distantPast, b = $1.modificationDate ?? .distantPast
                if a != b { return a > b }
                return $0.pageIndex == $1.pageIndex ? $0.annotationIndex < $1.annotationIndex : $0.pageIndex < $1.pageIndex
            }
        }
        annotationRows = rows
        selectedAnnotationRow = selected.flatMap { item in
            rows.firstIndex { $0.documentID == item.documentID && $0.pageID == item.pageID && $0.annotationID == item.annotationID }
        }
    }
    public func show(_ mode: ReaderSidebarMode) { self.mode = mode; _ = view; refresh() }
    /// Call after document mutation and from ReaderSession.onChange. Does not replace the host callback.
    public func refresh() {
        guard isViewLoaded else { return }
        modes.selectedSegment = mode.rawValue
        refreshAnnotations()
        thumbnailView.isHidden = mode != .thumbnails
        scroll.isHidden = mode == .thumbnails || isAnnotationEmptyStateVisible
        annotationHeader.isHidden = mode != .annotations
        annotationEmptyView.isHidden = !isAnnotationEmptyStateVisible
        status.isHidden = mode == .annotations
        table.rowHeight = mode == .annotations ? 78 : 54
        table.setAccessibilityLabel(mode == .annotations ? "批注列表" : "大纲或搜索结果列表")
        updateAnnotationTools()
        searchField.isHidden = mode != .search; searchNavigation.isHidden = mode != .search
        searchField.isEnabled = session.allowsCopying && session.pdfView.document != nil
        outlineRows = session.pdfView.document.map(ReaderOutlineItem.project) ?? []
        if mode == .search { status.stringValue = session.searchStatus }
        else if mode == .outline { status.stringValue = outlineRows.isEmpty ? "此文档没有大纲" : "\(outlineRows.count) 项 · 外部目标不会打开" }
        else { status.stringValue = "\(session.pdfView.document?.pageCount ?? 0) 页" }
        syncSelection = true
        table.reloadData()
        if mode == .search, let index = session.selectedHit, session.hits.indices.contains(index) {
            table.selectRowIndexes(IndexSet(integer: index), byExtendingSelection: false)
            table.scrollRowToVisible(index)
        } else if mode == .annotations, let row = selectedAnnotationRow {
            table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        } else { table.deselectAll(nil) }
        syncSelection = false
        updateAnnotationDeletion()
    }
    public func navigateToOutlineRow(_ row: Int) {
        guard outlineRows.indices.contains(row), let destination = outlineRows[row].destination,
              let page = destination.page, let document = session.pdfView.document,
              document.index(for: page) != NSNotFound else { return }
        session.pdfView.go(to: destination)
    }
    public func numberOfRows(in tableView: NSTableView) -> Int {
        switch mode {
        case .outline: return outlineRows.count
        case .search: return session.hits.count
        case .annotations: return annotationRows.count
        case .thumbnails: return 0
        }
    }
    public func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        if mode == .annotations, annotationRows.indices.contains(row) { return annotationCell(annotationRows[row]) }
        let label = NSTextField(wrappingLabelWithString: "")
        label.font = .systemFont(ofSize: 12); label.maximumNumberOfLines = 2; label.lineBreakMode = .byTruncatingTail
        if mode == .outline, outlineRows.indices.contains(row) {
            let item = outlineRows[row]
            label.stringValue = String(repeating: "  ", count: min(12, item.depth)) + item.title + (item.pageIndex.map { "   · \($0 + 1)" } ?? "")
            label.textColor = item.destination == nil ? .secondaryLabelColor : .labelColor
        } else if mode == .search, session.hits.indices.contains(row) {
            let hit = session.hits[row]; label.stringValue = "第\(hit.pageIndex + 1)页 [\(hit.pageLabel)]\n\(hit.excerpt)"
        }
        return label
    }
    public func tableViewSelectionDidChange(_ notification: Notification) {
        guard !syncSelection else { return }
        if mode == .outline { navigateToOutlineRow(table.selectedRow) }
        else if mode == .search { session.selectHit(table.selectedRow) }
        else if mode == .annotations {
            if table.selectedRow < 0 { selectedAnnotationRow = nil; updateAnnotationDeletion() }
            else { navigateToAnnotationRow(table.selectedRow) }
        }
    }
    public func controlTextDidChange(_ obj: Notification) {
        guard obj.object as? NSSearchField === searchField else { return }
        do { try session.search(searchField.stringValue) } catch { status.stringValue = "搜索不可用：\(error)" }
    }
    @objc private func changeSidebarMode() { if let mode = ReaderSidebarMode(rawValue: modes.selectedSegment) { show(mode) } }
    @objc private func previousHit() { session.nextHit(backward: true); refresh() }
    @objc private func nextHit() { session.nextHit(); refresh() }
    @objc private func cancel() { session.cancelSearch(); refresh() }
    private func annotationCell(_ item: ReaderAnnotationItem) -> NSView {
        let cell = NSView()
        let dot = NSView(); dot.wantsLayer = true
        dot.layer?.backgroundColor = item.color.cgColor; dot.layer?.cornerRadius = 4
        let heading = NSTextField(labelWithString: "第 \(item.pageIndex + 1) 页 · \(item.typeTitle)")
        heading.font = .systemFont(ofSize: 11, weight: .medium)
        heading.lineBreakMode = .byTruncatingTail
        let detail = NSTextField(wrappingLabelWithString: [item.contents, item.excerpt].filter { !$0.isEmpty }.joined(separator: "\n"))
        detail.font = .systemFont(ofSize: 11); detail.maximumNumberOfLines = 3
        detail.textColor = .secondaryLabelColor; detail.lineBreakMode = .byTruncatingTail
        for child in [dot, heading, detail] { child.translatesAutoresizingMaskIntoConstraints = false; cell.addSubview(child) }
        NSLayoutConstraint.activate([
            dot.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 6), dot.topAnchor.constraint(equalTo: cell.topAnchor, constant: 10),
            dot.widthAnchor.constraint(equalToConstant: 8), dot.heightAnchor.constraint(equalToConstant: 8),
            heading.leadingAnchor.constraint(equalTo: dot.trailingAnchor, constant: 6), heading.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -6),
            heading.topAnchor.constraint(equalTo: cell.topAnchor, constant: 6),
            detail.leadingAnchor.constraint(equalTo: heading.leadingAnchor), detail.trailingAnchor.constraint(equalTo: heading.trailingAnchor),
            detail.topAnchor.constraint(equalTo: heading.bottomAnchor, constant: 4), detail.bottomAnchor.constraint(lessThanOrEqualTo: cell.bottomAnchor, constant: -4)
        ])
        cell.setAccessibilityLabel(heading.stringValue + " " + detail.stringValue)
        return cell
    }
    @objc private func activateAnnotationTool(_ sender: ToolStripButton) { onAnnotationTool?(sender.toolID) }
    @objc private func showAnnotationOptions(_ sender: NSButton) {
        let menu = NSMenu(); menu.autoenablesItems = false
        for (title, action, checked) in [("按页码排序", #selector(sortAnnotationsByPage), annotationSort == .page),
                                          ("最新修改在前", #selector(sortAnnotationsByDate), annotationSort == .newestFirst)] {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: ""); item.target = self
            item.state = checked ? .on : .off; menu.addItem(item)
        }
        menu.addItem(.separator())
        let copy = NSMenuItem(title: "复制批注摘要", action: #selector(copyAnnotationSummary), keyEquivalent: "")
        copy.target = self; copy.isEnabled = session.allowsCopying && !annotationRows.isEmpty; menu.addItem(copy)
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: sender.bounds.minY), in: sender)
    }
    @objc private func sortAnnotationsByPage() { setAnnotationSort(.page) }
    @objc private func sortAnnotationsByDate() { setAnnotationSort(.newestFirst) }
    @objc private func copyAnnotationSummary() {
        refresh()
        guard !annotationSummary.isEmpty else { return }
        NSPasteboard.general.clearContents(); NSPasteboard.general.setString(annotationSummary, forType: .string)
    }
}

/// Original vector illustration: a sheet, comment bubble and pencil.
private final class AnnotationEmptyGlyphView: NSView {
    override func draw(_ dirtyRect: NSRect) {
        NSColor(calibratedWhite: 0.45, alpha: 1).setStroke()
        let sheet = NSBezierPath(roundedRect: NSRect(x: 22, y: 16, width: 52, height: 66), xRadius: 5, yRadius: 5)
        sheet.lineWidth = 2; sheet.stroke()
        let lines = NSBezierPath(); lines.lineWidth = 2
        for y in [62, 51, 40] { lines.move(to: NSPoint(x: 33, y: y)); lines.line(to: NSPoint(x: 60, y: y)) }
        lines.stroke()
        NSColor(calibratedWhite: 0.20, alpha: 1).setFill()
        let bubble = NSBezierPath(roundedRect: NSRect(x: 5, y: 65, width: 29, height: 20), xRadius: 5, yRadius: 5)
        bubble.fill(); bubble.stroke()
        let pencil = NSBezierPath(); pencil.move(to: NSPoint(x: 58, y: 17)); pencil.line(to: NSPoint(x: 81, y: 48))
        pencil.line(to: NSPoint(x: 87, y: 43)); pencil.line(to: NSPoint(x: 64, y: 12)); pencil.close()
        pencil.fill(); pencil.stroke()
    }
}

public enum ReaderCommand: String, CaseIterable {
    case find, findNext, findPrevious, findSelection
    case singlePage, twoPages, cover, fitPage, fitWidth, actualSize, zoomIn, zoomOut
    case outline, thumbnails, toggleSidebar, newTab, nextTab, previousTab, closeOtherTabs
}
public struct ReaderShortcut {
    public let key: String
    public let modifiers: NSEvent.ModifierFlags
    public static func forCommand(_ command: ReaderCommand) -> ReaderShortcut {
        let key: String
        var modifiers: NSEvent.ModifierFlags = .command
        switch command {
        case .find: key = "f"
        case .findNext: key = "g"
        case .findPrevious: key = "g"; modifiers.insert(.shift)
        case .findSelection: key = "e"
        case .singlePage: key = "1"
        case .twoPages: key = "2"
        case .cover: key = "3"
        case .fitPage: key = "8"
        case .fitWidth: key = "9"
        case .actualSize: key = "0"
        case .zoomIn: key = "+"
        case .zoomOut: key = "-"
        case .outline: key = "2"; modifiers.insert(.option)
        case .thumbnails: key = "4"; modifiers.insert(.option)
        case .toggleSidebar: key = "0"; modifiers.insert(.option)
        case .newTab: key = "t"
        case .nextTab: key = "]"; modifiers.insert(.shift)
        case .previousTab: key = "["; modifiers.insert(.shift)
        case .closeOtherTabs: key = "w"; modifiers.insert(.option)
        }
        return .init(key: key, modifiers: modifiers)
    }
}

public enum ReaderTabPolicy {
    public static func adjacent<ID: Equatable>(ids: [ID], active: ID?, backward: Bool) -> ID? {
        guard !ids.isEmpty else { return nil }
        guard let active, let index = ids.firstIndex(of: active) else { return backward ? ids.last : ids.first }
        return ids[(index + (backward ? -1 : 1) + ids.count) % ids.count]
    }
    /// Returns IDs to close only after every guard approves. Caller performs removal as a separate phase.
    /// Approval may show save prompts but must not remove tabs; cancellation keeps the tab set intact.
    public static func closeOthers<ID: Equatable>(ids: [ID], keeping: ID, approve: (ID) -> Bool) -> [ID]? {
        guard ids.contains(keeping) else { return nil }
        let candidates = ids.filter { $0 != keeping }
        guard candidates.allSatisfy(approve) else { return nil }
        return candidates
    }
}
