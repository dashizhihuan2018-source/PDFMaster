import AppKit

/// Keep the user's preferred width separate from temporary narrow-window clamping.
@MainActor final class ReaderSidebarWidth {
    static let key = "reader.sidebar.width.v1"
    static let minimum: CGFloat = 176
    static let maximum: CGFloat = 480
    private let defaults: UserDefaults
    private(set) var preferred: CGFloat

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        let saved = defaults.double(forKey: Self.key)
        preferred = saved.isFinite && saved >= Self.minimum && saved <= Self.maximum ? saved : Self.minimum
    }

    func fitted(_ proposed: CGFloat? = nil, available: CGFloat) -> CGFloat {
        let upper = max(0, min(Self.maximum, available - 200))
        let lower = min(Self.minimum, upper)
        let value = proposed ?? preferred
        return min(upper, max(lower, value.isFinite ? value : preferred))
    }

    func commit(_ width: CGFloat) {
        guard width.isFinite else { return }
        preferred = min(Self.maximum, max(Self.minimum, width))
        defaults.set(Double(preferred), forKey: Self.key)
    }
}

/// An AppKit responder, shared by all four native reader sidebar modes.
@MainActor final class ReaderSidebarDivider: NSView {
    var currentWidth: (() -> CGFloat)?
    var onWidth: ((CGFloat, Bool) -> Void)?
    var onCancel: (() -> Void)?
    private var dragStart: (x: CGFloat, width: CGFloat)?
    private var resignObserver: NSObjectProtocol?
    override var acceptsFirstResponder: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        identifier = NSUserInterfaceItemIdentifier("reader-sidebar-divider")
        toolTip = "拖动调整左栏宽度；方向键微调，Esc 取消拖动"
        setAccessibilityElement(true)
        setAccessibilityRole(.splitter)
        setAccessibilityLabel("左栏宽度调节")
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func resetCursorRects() { addCursorRect(bounds, cursor: .resizeLeftRight) }
    override func draw(_ dirtyRect: NSRect) {
        NSColor.separatorColor.setFill()
        NSRect(x: bounds.midX, y: bounds.minY, width: 1, height: bounds.height).fill()
    }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if let resignObserver { NotificationCenter.default.removeObserver(resignObserver) }
        resignObserver = nil
        if let window {
            resignObserver = NotificationCenter.default.addObserver(forName: NSWindow.didResignKeyNotification, object: window, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.cancelDrag() }
            }
        } else { cancelDrag() }
    }
    deinit { if let resignObserver { NotificationCenter.default.removeObserver(resignObserver) } }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        dragStart = (event.locationInWindow.x, currentWidth?() ?? ReaderSidebarWidth.minimum)
    }
    override func mouseDragged(with event: NSEvent) {
        guard let start = dragStart else { return }
        onWidth?(start.width + event.locationInWindow.x - start.x, false)
    }
    override func mouseUp(with event: NSEvent) {
        guard let start = dragStart else { return }
        dragStart = nil
        onWidth?(start.width + event.locationInWindow.x - start.x, true)
    }
    private func cancelDrag() {
        guard dragStart != nil else { return }
        dragStart = nil; onCancel?()
    }
    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53 { cancelDrag(); return }
        let width = currentWidth?() ?? ReaderSidebarWidth.minimum
        let step: CGFloat = event.modifierFlags.contains(.shift) ? 32 : 8
        switch event.keyCode {
        case 123: onWidth?(width - step, true)
        case 124: onWidth?(width + step, true)
        case 115: onWidth?(ReaderSidebarWidth.minimum, true)
        case 119: onWidth?(ReaderSidebarWidth.maximum, true)
        default: super.keyDown(with: event)
        }
    }
}
