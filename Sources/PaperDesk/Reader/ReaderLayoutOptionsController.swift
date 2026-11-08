import AppKit

private final class ReaderLayoutChoiceButton: NSButton {
    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard state == .on else { return }
        let border = NSBezierPath(roundedRect: bounds.insetBy(dx: 1, dy: 1), xRadius: 7, yRadius: 7)
        border.lineWidth = 2
        NSColor.systemBlue.withAlphaComponent(isEnabled ? 1 : 0.45).setStroke()
        border.stroke()
    }
}

/// Layout controls route through the same session as the menu and shortcuts.
@MainActor final class ReaderLayoutOptionsController: NSViewController {
    private let commands: ReaderCommandController
    let singleButton: NSButton = ReaderLayoutChoiceButton(title: "单页", target: nil, action: nil)
    let doubleButton: NSButton = ReaderLayoutChoiceButton(title: "双页", target: nil, action: nil)
    let coverButton = NSButton(checkboxWithTitle: "单独显示首页", target: nil, action: nil)
    let continuousButton = NSButton(checkboxWithTitle: "连续滚动", target: nil, action: nil)
    let fitButton = NSButton(title: "适合页面", target: nil, action: nil)

    init(commands: ReaderCommandController) {
        self.commands = commands
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 176, height: 230))
        let stack = NSStackView(); stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 14
        stack.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(stack)
        let title = NSTextField(labelWithString: "布局选项"); title.font = .boldSystemFont(ofSize: 14)
        stack.addArrangedSubview(title)
        stack.addArrangedSubview(NSTextField(labelWithString: "页面布局"))
        let row = NSStackView(views: [singleButton, doubleButton]); row.spacing = 8; row.distribution = .fillEqually
        for (button, symbol) in [(singleButton, "doc"), (doubleButton, "book") ] {
            button.setButtonType(.toggle); button.isBordered = false
            button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
            button.imagePosition = .imageLeading
            button.heightAnchor.constraint(equalToConstant: 36).isActive = true
            button.target = self; button.action = #selector(selectLayout(_:))
        }
        stack.addArrangedSubview(row)
        row.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        coverButton.target = self; coverButton.action = #selector(toggleCover)
        continuousButton.target = self; continuousButton.action = #selector(toggleContinuous)
        stack.addArrangedSubview(coverButton)
        continuousButton.isHidden = true
        fitButton.target = self; fitButton.action = #selector(fitSpread)
        fitButton.bezelStyle = .rounded; fitButton.toolTip = "将当前单页或双页完整显示在窗口内"
        fitButton.isHidden = true
        let trailing = stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -16)
        // The sidebar host is temporarily zero-width while page overview or a
        // hidden sidebar is laying out. Let the clipped panel keep its intrinsic
        // content until the persisted sidebar width is restored.
        trailing.priority = .defaultHigh
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 16),
            trailing,
            stack.topAnchor.constraint(equalTo: view.topAnchor, constant: 16)
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
        singleButton.contentTintColor = enabled ? (layout == .single ? .systemBlue : .labelColor) : .disabledControlTextColor
        doubleButton.contentTintColor = enabled ? (layout == .single ? .labelColor : .systemBlue) : .disabledControlTextColor
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
