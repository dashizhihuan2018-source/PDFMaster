import AppKit

struct ReferenceStripTool {
    let id: String; let title: String; let symbol: String
    var showsTitle = false
    var available = true
}

enum ReferenceToolStrip {
    static func tools(for mode: WorkbenchMode) -> [ReferenceStripTool] {
        switch mode {
        case .annotation, .drawing:
            return [
                .init(id: "highlight", title: "高亮", symbol: "highlighter"),
                .init(id: "underline", title: "下划线", symbol: "underline"),
                .init(id: "strikeout", title: "删除线", symbol: "strikethrough"),
                .init(id: "pen", title: "手绘", symbol: "pencil.tip.crop.circle"),
                .init(id: "eraser", title: "橡皮（整条笔迹）", symbol: "eraser"),
                .init(id: "text", title: "文字批注", symbol: "character.textbox"),
                .init(id: "shapes", title: "形状", symbol: "square.on.circle"),
                .init(id: "note", title: "便笺", symbol: "ellipsis.bubble"),
                .init(id: "stamp", title: "印章", symbol: "seal", available: false),
                .init(id: "signature", title: "签名", symbol: "signature"),
                .init(id: "selection", title: "选择批注", symbol: "rectangle.dashed")
            ]
        case .editing:
            return [
                .init(id: "content-text", title: "文本", symbol: "text.cursor", showsTitle: true),
                .init(id: "content-image", title: "图像", symbol: "photo", showsTitle: true),
                .init(id: "content-link", title: "链接", symbol: "link", showsTitle: true),
                .init(id: "content-redact", title: "隐藏", symbol: "doc.text", showsTitle: true)
            ]
        case .pages:
            return [
                .init(id: "insert", title: "插入页", symbol: "doc.badge.plus", showsTitle: true),
                .init(id: "attach", title: "附加文件", symbol: "doc.on.doc", showsTitle: true),
                .init(id: "rotate", title: "旋转", symbol: "rotate.right", showsTitle: true),
                .init(id: "copy-page", title: "复制", symbol: "doc.on.doc", showsTitle: true),
                .init(id: "paste-page", title: "粘贴", symbol: "doc.on.clipboard", showsTitle: true, available: false),
                .init(id: "extract", title: "摘录", symbol: "square.and.arrow.up", showsTitle: true),
                .init(id: "delete-page", title: "删除", symbol: "trash", showsTitle: true)
            ]
        case .signature:
            return [.init(id: "signature", title: "签名", symbol: "signature", showsTitle: true), .init(id: "text", title: "文字", symbol: "textformat", showsTitle: true)]
        case .export:
            return [.init(id: "save", title: "存储副本", symbol: "square.and.arrow.down", showsTitle: true), .init(id: "password", title: "密码保护", symbol: "lock", showsTitle: true), .init(id: "export-text", title: "纯文本", symbol: "doc.text", showsTitle: true)]
        }
    }
}

/// Shared native-button feedback; entering a control never changes selection or sends its action.
class HoverFeedbackButton: NSButton {
    enum Feedback { case normal, hover, pressed, selected, disabled }
    private(set) var isPointerInside = false
    private var hoverTrackingArea: NSTrackingArea?
    var feedback: Feedback {
        if !isEnabled { return .disabled }
        if isHighlighted { return .pressed }
        if state == .on { return .selected }
        return isPointerInside ? .hover : .normal
    }
    var feedbackCornerRadius: CGFloat { 6 }
    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        focusRingType = .exterior
    }
    required init?(coder: NSCoder) {
        super.init(coder: coder)
        focusRingType = .exterior
    }
    override var isEnabled: Bool {
        didSet {
            if !isEnabled { isPointerInside = false; highlight(false) }
            needsDisplay = true
            noteFocusRingMaskChanged()
        }
    }
    override var state: NSControl.StateValue { didSet { needsDisplay = true } }
    override var toolTip: String? { didSet { setAccessibilityHelp(toolTip) } }
    override var acceptsFirstResponder: Bool { isEnabled }
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let hoverTrackingArea { removeTrackingArea(hoverTrackingArea) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self, userInfo: nil)
        addTrackingArea(area); hoverTrackingArea = area
    }
    override func mouseEntered(with event: NSEvent) {
        guard isEnabled else { return }
        isPointerInside = true; needsDisplay = true
    }
    override func mouseExited(with event: NSEvent) {
        isPointerInside = false; needsDisplay = true
    }
    override func highlight(_ flag: Bool) {
        super.highlight(flag && isEnabled); needsDisplay = true
    }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { isPointerInside = false; needsDisplay = true }
    }
    override var focusRingMaskBounds: NSRect { bounds }
    override func drawFocusRingMask() {
        NSBezierPath(roundedRect: bounds, xRadius: feedbackCornerRadius, yRadius: feedbackCornerRadius).fill()
    }
}

final class ToolStripButton: HoverFeedbackButton {
    let toolID: String
    init(tool: ReferenceStripTool, target: AnyObject?, action: Selector?) {
        toolID = tool.id
        super.init(frame: .zero)
        title = tool.showsTitle ? tool.title : ""
        image = NSImage(systemSymbolName: tool.symbol, accessibilityDescription: tool.title)?.withSymbolConfiguration(.init(pointSize: 16, weight: .regular))
        imagePosition = tool.showsTitle ? .imageLeft : .imageOnly
        self.target = target; self.action = action
        isBordered = false; bezelStyle = .inline; font = .systemFont(ofSize: 11)
        contentTintColor = tool.id == "delete-page" ? .systemRed : ShellPalette.primary
        isEnabled = tool.available; toolTip = tool.available ? tool.title : "\(tool.title)：暂不支持"
        setAccessibilityLabel("工具 · \(tool.title)")
        setAccessibilityHelp(toolTip)
        heightAnchor.constraint(equalToConstant: 28).isActive = true
        if !tool.showsTitle { widthAnchor.constraint(equalToConstant: 28).isActive = true }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func draw(_ dirtyRect: NSRect) {
        let fill: NSColor?
        switch feedback {
        case .hover: fill = NSColor(srgbRed: 41.0 / 255, green: 41.0 / 255, blue: 41.0 / 255, alpha: 1)
        case .pressed: fill = NSColor(srgbRed: 0.08, green: 0.13, blue: 0.2, alpha: 1)
        case .selected: fill = NSColor(srgbRed: 0.12, green: 0.2, blue: 0.3, alpha: 1)
        case .normal, .disabled: fill = nil
        }
        if let fill {
            fill.setFill()
            NSBezierPath(roundedRect: bounds, xRadius: feedbackCornerRadius, yRadius: feedbackCornerRadius).fill()
        }
        super.draw(dirtyRect)
    }
}
