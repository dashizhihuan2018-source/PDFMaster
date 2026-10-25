import AppKit

/// Layout controls route through the same session as the menu and shortcuts.
@MainActor final class ReaderLayoutOptionsController: NSViewController {
    private let commands: ReaderCommandController
    let singleButton = NSButton(title: "单页", target: nil, action: nil)
    let doubleButton = NSButton(title: "双页", target: nil, action: nil)
    let coverButton = NSButton(checkboxWithTitle: "单独显示首页", target: nil, action: nil)
    let continuousButton = NSButton(checkboxWithTitle: "连续滚动", target: nil, action: nil)

    init(commands: ReaderCommandController) {
        self.commands = commands
        super.init(nibName: nil, bundle: nil)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 280, height: 230))
        let stack = NSStackView(); stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 14
        stack.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(stack)
        let title = NSTextField(labelWithString: "布局选项"); title.font = .boldSystemFont(ofSize: 14)
        stack.addArrangedSubview(title)
        stack.addArrangedSubview(NSTextField(labelWithString: "页面布局"))
        let row = NSStackView(views: [singleButton, doubleButton]); row.spacing = 12
        for (button, symbol) in [(singleButton, "doc"), (doubleButton, "book") ] {
            button.setButtonType(.toggle); button.bezelStyle = .rounded
            button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
            button.imagePosition = .imageLeading
            button.widthAnchor.constraint(equalToConstant: 112).isActive = true
            button.target = self; button.action = #selector(selectLayout(_:))
        }
        stack.addArrangedSubview(row)
        coverButton.target = self; coverButton.action = #selector(toggleCover)
        continuousButton.target = self; continuousButton.action = #selector(toggleContinuous)
        stack.addArrangedSubview(coverButton); stack.addArrangedSubview(continuousButton)
        let fit = NSButton(title: "适合页面", target: self, action: #selector(fitSpread))
        fit.bezelStyle = .rounded; fit.toolTip = "将当前单页或双页完整显示在窗口内"
        stack.addArrangedSubview(fit)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 16),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -16),
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
