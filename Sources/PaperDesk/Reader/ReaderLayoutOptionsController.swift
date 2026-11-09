import AppKit

/// Independently drawn vector controls; no assets copied from the reference app.
/// NSButtonCell retains native tracking, keyboard activation and accessibility.
private final class ReaderLayoutChoiceCell: NSButtonCell {
    private var iconWidth: CGFloat { title == "单页" ? 12 : 18 }
    private var textSize: NSSize { (title as NSString).size(withAttributes: [.font: font ?? .systemFont(ofSize: 13)]) }
    override func imageRect(forBounds rect: NSRect) -> NSRect {
        let width = iconWidth + 6 + textSize.width
        return NSRect(x: rect.midX - width / 2, y: rect.midY - 8.5, width: iconWidth, height: 17)
    }
    override func titleRect(forBounds rect: NSRect) -> NSRect {
        let icon = imageRect(forBounds: rect)
        return NSRect(x: icon.maxX + 6, y: rect.midY - textSize.height / 2, width: textSize.width, height: textSize.height)
    }
    override func draw(withFrame cellFrame: NSRect, in controlView: NSView) {
        let dark = controlView.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        if state == .on || isHighlighted {
            let path = NSBezierPath(roundedRect: cellFrame.insetBy(dx: 0.5, dy: 0.5), xRadius: 3, yRadius: 3)
            NSColor(srgbRed: dark ? 0.23 : 0.9, green: dark ? 0.23 : 0.9, blue: dark ? 0.25 : 0.9, alpha: isEnabled ? 1 : 0.4).setFill()
            path.fill()
            if state == .on {
                NSColor(srgbRed: 0.25, green: 0.59, blue: 0.86, alpha: isEnabled ? 1 : 0.4).setStroke()
                path.lineWidth = 1; path.stroke()
            }
        }
        let gray: CGFloat = dark ? 0.76 : 0.3
        let foreground = isEnabled ? NSColor(srgbRed: gray, green: gray, blue: gray, alpha: 1) : .disabledControlTextColor
        (title as NSString).draw(in: titleRect(forBounds: cellFrame), withAttributes: [.font: font ?? .systemFont(ofSize: 13), .foregroundColor: foreground])
        let icon = imageRect(forBounds: cellFrame)
        let path = NSBezierPath(); path.lineWidth = 1; path.lineCapStyle = .round; path.lineJoinStyle = .round
        func move(_ x: CGFloat, _ y: CGFloat) { path.move(to: NSPoint(x: icon.minX + x, y: icon.minY + y)) }
        func line(_ x: CGFloat, _ y: CGFloat) { path.line(to: NSPoint(x: icon.minX + x, y: icon.minY + y)) }
        if title == "单页" {
            path.appendRoundedRect(NSRect(x: icon.minX + 0.5, y: icon.minY + 0.5, width: 11, height: 16), xRadius: 2, yRadius: 2)
            for y: CGFloat in [5, 7.5] { move(3, y); line(9, y) }
        } else {
            move(9, 2); line(9, 16)
            move(9, 2); path.curve(to: NSPoint(x: icon.minX + 0.5, y: icon.minY + 0.5), controlPoint1: NSPoint(x: icon.minX + 6, y: icon.minY), controlPoint2: NSPoint(x: icon.minX + 3, y: icon.minY))
            line(0.5, 14.5); path.curve(to: NSPoint(x: icon.minX + 9, y: icon.minY + 16.5), controlPoint1: NSPoint(x: icon.minX + 4, y: icon.minY + 14), controlPoint2: NSPoint(x: icon.minX + 6, y: icon.minY + 15))
            path.curve(to: NSPoint(x: icon.minX + 17.5, y: icon.minY + 14.5), controlPoint1: NSPoint(x: icon.minX + 12, y: icon.minY + 15), controlPoint2: NSPoint(x: icon.minX + 14, y: icon.minY + 14))
            line(17.5, 0.5); path.curve(to: NSPoint(x: icon.minX + 9, y: icon.minY + 2), controlPoint1: NSPoint(x: icon.minX + 15, y: icon.minY), controlPoint2: NSPoint(x: icon.minX + 12, y: icon.minY))
            for y: CGFloat in [4.5, 7, 9.5] { move(2.5, y); line(6.5, y + 1); move(11.5, y + 1); line(15.5, y) }
        }
        foreground.setStroke(); path.stroke()
    }
}

private final class ReaderLayoutPanelView: NSView {
    override func draw(_ dirtyRect: NSRect) {
        let dark = effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        (dark ? NSColor(srgbRed: 40 / 255, green: 40 / 255, blue: 44 / 255, alpha: 1) : .windowBackgroundColor).setFill()
        dirtyRect.fill()
    }
}

/// Layout controls route through the same session as the menu and shortcuts.
@MainActor final class ReaderLayoutOptionsController: NSViewController {
    private let commands: ReaderCommandController
    let singleButton = NSButton(title: "单页", target: nil, action: nil)
    let doubleButton = NSButton(title: "双页", target: nil, action: nil)
    let coverButton = NSButton(checkboxWithTitle: "单独显示首页", target: nil, action: nil)
    let continuousButton = NSButton(checkboxWithTitle: "连续滚动", target: nil, action: nil)
    let fitButton = NSButton(title: "适合页面", target: nil, action: nil)

    init(commands: ReaderCommandController) {
        self.commands = commands
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func loadView() {
        view = ReaderLayoutPanelView(frame: NSRect(x: 0, y: 0, width: 240, height: 230))
        let title = NSTextField(labelWithString: "布局选项"); title.font = .boldSystemFont(ofSize: 13)
        let section = NSTextField(labelWithString: "页面布局"); section.font = .systemFont(ofSize: 11); section.textColor = .secondaryLabelColor
        let row = NSStackView(views: [singleButton, doubleButton]); row.spacing = 8; row.distribution = .fillEqually
        for button in [singleButton, doubleButton] {
            button.cell = ReaderLayoutChoiceCell(textCell: button.title)
            button.setButtonType(.toggle); button.isBordered = false
            button.font = .systemFont(ofSize: 13)
            button.setAccessibilityLabel(button.title)
            button.imagePosition = .imageLeading
            button.heightAnchor.constraint(equalToConstant: 28).isActive = true
            button.target = self; button.action = #selector(selectLayout(_:))
        }
        coverButton.target = self; coverButton.action = #selector(toggleCover)
        continuousButton.target = self; continuousButton.action = #selector(toggleContinuous)
        coverButton.font = .systemFont(ofSize: 12)
        coverButton.controlSize = .small
        continuousButton.isHidden = true
        fitButton.target = self; fitButton.action = #selector(fitSpread)
        fitButton.bezelStyle = .rounded; fitButton.toolTip = "将当前单页或双页完整显示在窗口内"
        fitButton.isHidden = true
        for child in [title, section, row, coverButton] { child.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(child) }
        let trailing = row.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -8)
        // The sidebar host is temporarily zero-width while page overview or a
        // hidden sidebar is laying out. Let the clipped panel keep its intrinsic
        // content until the persisted sidebar width is restored.
        trailing.priority = .defaultHigh
        NSLayoutConstraint.activate([
            title.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 14),
            title.topAnchor.constraint(equalTo: view.topAnchor, constant: 18),
            section.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            section.topAnchor.constraint(equalTo: view.topAnchor, constant: 54),
            row.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 8),
            trailing,
            row.topAnchor.constraint(equalTo: view.topAnchor, constant: 80),
            coverButton.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 16),
            coverButton.topAnchor.constraint(equalTo: row.bottomAnchor, constant: 18)
        ])
        refresh()
    }
    override func viewWillAppear() { super.viewWillAppear(); refresh() }
    func refresh() {
        guard isViewLoaded else { return }
        let layout = commands.layout
        singleButton.state = layout == .single ? .on : .off
        doubleButton.state = layout == .single ? .off : .on
        coverButton.isHidden = layout == .single
        coverButton.state = layout == .twoPagesWithCover ? .on : .off
        continuousButton.state = commands.continuous ? .on : .off
        let enabled = commands.canChangeLayout
        singleButton.isEnabled = enabled; doubleButton.isEnabled = enabled
        coverButton.isEnabled = enabled; continuousButton.isEnabled = enabled; fitButton.isEnabled = enabled
        singleButton.contentTintColor = enabled ? .labelColor : .disabledControlTextColor
        doubleButton.contentTintColor = enabled ? .labelColor : .disabledControlTextColor
        singleButton.needsDisplay = true; doubleButton.needsDisplay = true
    }
    @objc private func selectLayout(_ sender: NSButton) {
        _ = commands.run(sender === singleButton ? .singlePage : .twoPages)
        refresh()
    }
    @objc private func toggleCover() {
        _ = commands.run(coverButton.state == .on ? .cover : .twoPages)
        refresh()
    }
    @objc private func toggleContinuous() { commands.setContinuous(continuousButton.state == .on); refresh() }
    @objc private func fitSpread() { _ = commands.run(.fitPage); refresh() }
}
