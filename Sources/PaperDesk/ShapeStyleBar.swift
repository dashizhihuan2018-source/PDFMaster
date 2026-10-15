import AppKit
import PaperCore

enum ShapeToolKind: String, CaseIterable {
    case rectangle, ellipse, line, arrow, doubleArrow

    var supportsFill: Bool { self == .rectangle || self == .ellipse }
    var title: String {
        switch self {
        case .rectangle: return "矩形"
        case .ellipse: return "椭圆"
        case .line: return "直线"
        case .arrow: return "箭头"
        case .doubleArrow: return "双向箭头"
        }
    }
    var symbol: String {
        switch self {
        case .rectangle: return "rectangle"
        case .ellipse: return "circle"
        case .line: return "line.diagonal"
        case .arrow: return "arrow.up.right"
        case .doubleArrow: return "arrow.up.right.and.arrow.down.left"
        }
    }
}

enum ShapeStyleBarMode { case drawing, selection }

@MainActor final class ShapeWidthSlider: NSSlider {
    private(set) var isShapeTracking = false
    var onTrackingBegin: (() -> Void)?
    var onTrackingEnd: (() -> Void)?
    private var mouseTracking: (axis: CGFloat, value: Double, span: CGFloat)?
    override var mouseDownCanMoveWindow: Bool { false }

    override var isEnabled: Bool {
        didSet { if !isEnabled { cancelShapeTracking() } }
    }

    func beginShapeTracking() {
        guard isEnabled, !isShapeTracking else { return }
        isShapeTracking = true
        onTrackingBegin?()
    }

    func endShapeTracking() {
        guard isShapeTracking else { return }
        mouseTracking = nil
        isShapeTracking = false
        cell?.isHighlighted = false
        needsDisplay = true
        onTrackingEnd?()
    }

    func cancelShapeTracking() {
        mouseTracking = nil
        isShapeTracking = false
        cell?.isHighlighted = false
        needsDisplay = true
    }

    override func mouseDown(with event: NSEvent) {
        guard isEnabled, !isShapeTracking, let cell = cell as? NSSliderCell,
              sliderType == .linear, minValue.isFinite, maxValue.isFinite, maxValue > minValue else { return }
        let point = convert(event.locationInWindow, from: nil)
        guard bounds.contains(point) else { return }
        cell.calcDrawInfo(bounds)
        let knob = cell.knobRect(flipped: isFlipped)
        let bar = cell.barRect(flipped: isFlipped)
        guard !bar.isEmpty, !knob.isEmpty else { return }

        // The native bar includes the knob's full travel. Inset its endpoints
        // by half the native knob size, never by an assumed control padding.
        // knobRect is AppKit's last drawn rect; changing doubleValue alone does
        // not update it, so querying it at temporary min/max values is invalid.
        let halfKnob = (isVertical ? knob.height : knob.width) / 2
        let lower = (isVertical ? bar.minY : bar.minX) + halfKnob
        let upper = (isVertical ? bar.maxY : bar.maxX) - halfKnob
        let reversed = isVertical ? isFlipped : userInterfaceLayoutDirection == .rightToLeft
        let start = reversed ? upper : lower
        let end = reversed ? lower : upper
        let span = end - start
        let axis = isVertical ? point.y : point.x
        guard span.isFinite, abs(span) > 0, axis.isFinite else { return }

        window?.makeFirstResponder(self)
        beginShapeTracking()
        cell.isHighlighted = true
        if !knob.contains(point) {
            let fraction = min(1, max(0, Double((axis - start) / span)))
            updateMouseValue(minValue + fraction * (maxValue - minValue))
        }
        guard isEnabled, isShapeTracking else { return }
        // A knob grab anchors the exact current value, so an off-center click
        // or a stationary drag never introduces a rounding jump.
        mouseTracking = (axis, doubleValue, span)
        needsDisplay = true
    }

    override func mouseDragged(with event: NSEvent) {
        guard isEnabled, isShapeTracking, let tracking = mouseTracking else { return }
        let point = convert(event.locationInWindow, from: nil)
        let axis = isVertical ? point.y : point.x
        guard axis.isFinite else { return }
        updateMouseValue(tracking.value + Double((axis - tracking.axis) / tracking.span) * (maxValue - minValue))
    }

    override func mouseUp(with event: NSEvent) {
        guard isEnabled, isShapeTracking, mouseTracking != nil else { return }
        mouseDragged(with: event)
        endShapeTracking()
    }

    override func resignFirstResponder() -> Bool {
        let resigned = super.resignFirstResponder()
        if resigned { cancelShapeTracking() }
        return resigned
    }

    override func viewWillMove(toWindow newWindow: NSWindow?) {
        if window !== newWindow { cancelShapeTracking() }
        super.viewWillMove(toWindow: newWindow)
    }

    private func updateMouseValue(_ value: Double) {
        guard value.isFinite else { return }
        let next = min(maxValue, max(minValue, value))
        guard next != doubleValue else { return }
        doubleValue = next
        needsDisplay = true
        _ = sendAction(action, to: target)
    }
}

/// Inline, nonmodal shape controls. Configuration is silent; only user actions emit callbacks.
@MainActor
final class ShapeStyleBarView: NSView, NSTextFieldDelegate {
    var onToolChange: ((ShapeToolKind) -> Void)?
    var onStyleChange: ((AnnotationStyle) -> Void)?
    var onStylePreview: ((AnnotationStyle) -> Void)?
    private(set) var tool: ShapeToolKind = .rectangle
    private(set) var style: AnnotationStyle = .default
    private(set) var mode: ShapeStyleBarMode = .drawing

    let toolChooser = NSSegmentedControl()
    let strokeColorWell = NSColorWell()
    let fillEnabledButton = NSButton(checkboxWithTitle: "填充", target: nil, action: nil)
    let fillColorWell = NSColorWell()
    let widthSlider = ShapeWidthSlider(value: 1, minValue: 0.25, maxValue: 12, target: nil, action: nil)
    private var widthAtTrackingStart: CGFloat = 1
    let widthField = NSTextField(string: "1")
    let widthPresets = NSPopUpButton(frame: .zero, pullsDown: false)
    let transparencySlider = ShapeWidthSlider(value: 0, minValue: 0, maxValue: 100, target: nil, action: nil)
    let transparencyField = NSTextField(string: "0")
    private var alphaAtTrackingStart: CGFloat = 1
    private let modeLabel = NSTextField(labelWithString: "绘制")
    private var selectionTitle = "所选形状"
    private let preview = ShapeStyleExampleView()
    private let presetValues: [CGFloat] = [0.5, 1, 2, 3, 5, 8, 12]
    private var controlsEnabled = true
    // Keep the last area-shape fill while switching through line-family drawing tools.
    private var wantsFill = false
    private var rememberedFillColor = NSColor.systemYellow

    override init(frame frameRect: NSRect = .zero) {
        super.init(frame: frameRect)
        buildControls()
        widthSlider.onTrackingBegin = { [weak self] in if let self { self.widthAtTrackingStart = self.style.lineWidth } }
        widthSlider.onTrackingEnd = { [weak self] in
            guard let self, self.style.lineWidth != self.widthAtTrackingStart else { return }
            self.onStyleChange?(self.style)
        }
        transparencySlider.onTrackingBegin = { [weak self] in
            if let self { self.alphaAtTrackingStart = self.style.strokeColor.alphaComponent }
        }
        transparencySlider.onTrackingEnd = { [weak self] in
            guard let self, self.style.strokeColor.alphaComponent != self.alphaAtTrackingStart else { return }
            self.onStyleChange?(self.style)
        }
        reflectState()
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override var intrinsicContentSize: NSSize { NSSize(width: 880, height: 42) }

    /// Invalid programmatic widths retain the last valid width. Does not emit callbacks.
    func configure(tool: ShapeToolKind, style: AnnotationStyle,
                   mode: ShapeStyleBarMode = .drawing, enabled: Bool = true,
                   selectionTitle: String = "所选形状") {
        widthSlider.cancelShapeTracking()
        transparencySlider.cancelShapeTracking()
        self.tool = tool
        self.mode = mode
        self.selectionTitle = selectionTitle
        controlsEnabled = enabled
        self.style = AnnotationStyle(strokeColor: style.strokeColor,
            fillColor: tool.supportsFill ? style.fillColor : nil,
            lineWidth: Self.validWidth(style.lineWidth) ? style.lineWidth : self.style.lineWidth)
        if tool.supportsFill {
            wantsFill = style.fillColor != nil
            if let color = style.fillColor { rememberedFillColor = color }
        }
        reflectState()
    }

    private func buildControls() {
        setAccessibilityLabel("形状属性")
        toolChooser.segmentCount = ShapeToolKind.allCases.count
        toolChooser.trackingMode = .selectOne
        toolChooser.segmentStyle = .rounded
        toolChooser.controlSize = .small
        for (index, tool) in ShapeToolKind.allCases.enumerated() {
            let image = NSImage(systemSymbolName: tool.symbol, accessibilityDescription: tool.title)
            toolChooser.setImage(image, forSegment: index)
            if image == nil { toolChooser.setLabel(tool.title, forSegment: index) }
            toolChooser.setToolTip(tool.title, forSegment: index)
            toolChooser.setWidth(28, forSegment: index)
        }
        bind(toolChooser, label: "形状工具", action: #selector(changeTool(_:)))

        for (well, label, selector) in [
            (strokeColorWell, "描边颜色", #selector(changeStroke(_:))),
            (fillColorWell, "填充颜色", #selector(changeFillColor(_:)))
        ] {
            well.colorWellStyle = .minimal
            well.widthAnchor.constraint(equalToConstant: 28).isActive = true
            well.heightAnchor.constraint(equalToConstant: 24).isActive = true
            bind(well, label: label, action: selector)
        }
        fillEnabledButton.controlSize = .small
        bind(fillEnabledButton, label: "启用填充", action: #selector(changeFillEnabled(_:)))
        widthSlider.controlSize = .small
        widthSlider.isContinuous = true
        widthSlider.widthAnchor.constraint(equalToConstant: 80).isActive = true
        bind(widthSlider, label: "线宽滑块", action: #selector(changeWidthSlider(_:)))
        widthSlider.toolTip = "拖动调整线宽：0.25–12 点；输入框支持至 32 点"
        widthField.controlSize = .small
        widthField.font = .monospacedDigitSystemFont(ofSize: 11, weight: .regular)
        widthField.alignment = .right
        widthField.widthAnchor.constraint(equalToConstant: 47).isActive = true
        widthField.delegate = self
        bind(widthField, label: "线宽，单位点", action: #selector(changeWidthField(_:)))
        widthField.toolTip = "线宽（点）：输入 0.25–32 的有限数值，按回车应用"
        widthPresets.controlSize = .small
        for value in presetValues { widthPresets.addItem(withTitle: "\(Self.widthText(value)) pt") }
        widthPresets.widthAnchor.constraint(equalToConstant: 72).isActive = true
        bind(widthPresets, label: "预设线宽", action: #selector(changeWidthPreset(_:)))
        transparencySlider.controlSize = .small
        transparencySlider.isContinuous = true
        transparencySlider.widthAnchor.constraint(equalToConstant: 74).isActive = true
        bind(transparencySlider, label: "透明度滑块", action: #selector(changeTransparencySlider(_:)))
        transparencySlider.toolTip = "透明度：0% 完全不透明，100% 完全透明"
        transparencyField.controlSize = .small
        transparencyField.alignment = .right
        transparencyField.font = .monospacedDigitSystemFont(ofSize: 11, weight: .regular)
        transparencyField.widthAnchor.constraint(equalToConstant: 42).isActive = true
        transparencyField.delegate = self
        bind(transparencyField, label: "透明度，百分比", action: #selector(changeTransparencyField(_:)))
        transparencyField.toolTip = "输入 0–100，按回车应用"
        modeLabel.font = .systemFont(ofSize: 11, weight: .medium)
        modeLabel.textColor = .secondaryLabelColor
        modeLabel.widthAnchor.constraint(equalToConstant: 58).isActive = true
        preview.widthAnchor.constraint(equalToConstant: 48).isActive = true
        preview.heightAnchor.constraint(equalToConstant: 28).isActive = true

        let strokeLabel = label("描边")
        let widthLabel = label("线宽")
        let stack = NSStackView(views: [modeLabel, toolChooser, strokeLabel, strokeColorWell,
            fillEnabledButton, fillColorWell, widthLabel, widthSlider, widthField, widthPresets,
            label("透明度"), transparencySlider, transparencyField, label("%"), preview])
        stack.orientation = .horizontal
        stack.alignment = .centerY
        stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
            stack.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -10)
        ])
    }

    private func bind(_ control: NSControl, label: String, action: Selector) {
        control.target = self
        control.action = action
        control.setAccessibilityLabel(label)
        control.toolTip = label
    }

    private func label(_ text: String) -> NSTextField {
        let view = NSTextField(labelWithString: text)
        view.font = .systemFont(ofSize: 11)
        view.textColor = .secondaryLabelColor
        return view
    }

    private func reflectState() {
        toolChooser.selectedSegment = ShapeToolKind.allCases.firstIndex(of: tool) ?? 0
        toolChooser.isEnabled = controlsEnabled && mode == .drawing
        modeLabel.stringValue = mode == .drawing ? "绘制" : selectionTitle
        modeLabel.toolTip = mode == .drawing ? "在页面上拖动绘制新形状" : "直接修改所选形状的样式"
        strokeColorWell.color = style.strokeColor
        strokeColorWell.isEnabled = controlsEnabled
        fillEnabledButton.state = wantsFill ? .on : .off
        fillEnabledButton.isEnabled = controlsEnabled && tool.supportsFill
        fillColorWell.color = rememberedFillColor
        fillColorWell.isEnabled = controlsEnabled && tool.supportsFill && wantsFill
        widthSlider.isEnabled = controlsEnabled
        widthSlider.doubleValue = min(Double(style.lineWidth), 12)
        widthField.isEnabled = controlsEnabled
        widthField.stringValue = Self.widthText(style.lineWidth)
        widthPresets.isEnabled = controlsEnabled
        widthPresets.selectItem(at: presetValues.firstIndex(of: style.lineWidth) ?? -1)
        transparencySlider.isEnabled = controlsEnabled
        transparencyField.isEnabled = controlsEnabled
        let transparency = 100 * (1 - style.strokeColor.alphaComponent)
        transparencySlider.doubleValue = Double(transparency)
        transparencyField.stringValue = Self.widthText((transparency * 100).rounded() / 100)
        preview.tool = tool
        preview.style = style
        preview.needsDisplay = true
        preview.setAccessibilityValue("\(tool.title)，线宽 \(Self.widthText(style.lineWidth)) 点，\(style.fillColor == nil ? "无填充" : "有填充")")
    }

    private static func validWidth(_ width: CGFloat) -> Bool {
        width.isFinite && AnnotationStyle.supportedLineWidths.contains(width)
    }

    private static func widthText(_ width: CGFloat) -> String {
        // Do not round a real continuous slider value or a precisely entered PDF width.
        let text = String(Double(width))
        return text.hasSuffix(".0") ? String(text.dropLast(2)) : text
    }

    private func applyStyle(_ next: AnnotationStyle) {
        let changed = style.lineWidth != next.lineWidth || !style.strokeColor.isEqual(next.strokeColor)
            || style.fillColor != next.fillColor
        style = next
        reflectState()
        if changed {
            if widthSlider.isShapeTracking || transparencySlider.isShapeTracking { onStylePreview?(style) }
            else { onStyleChange?(style) }
        }
    }

    @objc private func changeTool(_ sender: NSSegmentedControl) {
        guard controlsEnabled, mode == .drawing,
              ShapeToolKind.allCases.indices.contains(sender.selectedSegment) else { reflectState(); return }
        let next = ShapeToolKind.allCases[sender.selectedSegment]
        guard next != tool else { return }
        tool = next
        style.fillColor = tool.supportsFill && wantsFill ? rememberedFillColor : nil
        reflectState()
        onToolChange?(tool)
    }

    @objc private func changeStroke(_ sender: NSColorWell) {
        guard controlsEnabled else { reflectState(); return }
        var next = style; next.strokeColor = sender.color.withAlphaComponent(style.strokeColor.alphaComponent); applyStyle(next)
    }

    @objc private func changeFillEnabled(_ sender: NSButton) {
        guard controlsEnabled, tool.supportsFill else { reflectState(); return }
        wantsFill = sender.state == .on
        var next = style; next.fillColor = wantsFill ? rememberedFillColor : nil; applyStyle(next)
    }

    @objc private func changeFillColor(_ sender: NSColorWell) {
        guard controlsEnabled, tool.supportsFill, wantsFill else { reflectState(); return }
        rememberedFillColor = sender.color.withAlphaComponent(style.strokeColor.alphaComponent)
        var next = style; next.fillColor = rememberedFillColor; applyStyle(next)
    }

    @objc private func changeWidthSlider(_ sender: NSSlider) {
        guard controlsEnabled else { reflectState(); return }
        applyWidth(CGFloat(sender.doubleValue))
    }

    @objc private func changeWidthField(_ sender: NSTextField) {
        guard controlsEnabled,
              let value = Double(sender.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)),
              Self.validWidth(CGFloat(value)) else {
            reflectState()
            return
        }
        applyWidth(CGFloat(value))
    }

    func controlTextDidEndEditing(_ notification: Notification) {
        if notification.object as? NSTextField === transparencyField {
            changeTransparencyField(transparencyField); return
        }
        guard notification.object as? NSTextField === widthField else { return }
        changeWidthField(widthField)
    }

    @objc private func changeWidthPreset(_ sender: NSPopUpButton) {
        guard controlsEnabled, presetValues.indices.contains(sender.indexOfSelectedItem) else { reflectState(); return }
        applyWidth(presetValues[sender.indexOfSelectedItem])
    }

    private func applyWidth(_ width: CGFloat) {
        guard Self.validWidth(width) else { reflectState(); return }
        var next = style; next.lineWidth = width; applyStyle(next)
    }

    @objc private func changeTransparencySlider(_ sender: NSSlider) {
        applyTransparency(sender.doubleValue)
    }

    @objc private func changeTransparencyField(_ sender: NSTextField) {
        guard let value = Double(sender.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)) else { reflectState(); return }
        applyTransparency(value)
    }

    private func applyTransparency(_ value: Double) {
        guard controlsEnabled, value.isFinite, (0...100).contains(value) else { reflectState(); return }
        let alpha = CGFloat(1 - value / 100)
        var next = style
        next.strokeColor = style.strokeColor.withAlphaComponent(alpha)
        rememberedFillColor = rememberedFillColor.withAlphaComponent(alpha)
        next.fillColor = style.fillColor?.withAlphaComponent(alpha)
        applyStyle(next)
    }
}

/// A native drawn example, not a substitute icon: updates with the exact current style.
@MainActor
private final class ShapeStyleExampleView: NSView {
    var tool: ShapeToolKind = .rectangle
    var style: AnnotationStyle = .default

    override init(frame frameRect: NSRect = .zero) {
        super.init(frame: frameRect)
        setAccessibilityElement(true)
        setAccessibilityRole(.image)
        setAccessibilityLabel("形状样式预览")
        toolTip = "当前形状的描边、填充和线宽预览"
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        NSColor.controlBackgroundColor.setFill()
        NSBezierPath(roundedRect: bounds, xRadius: 4, yRadius: 4).fill()
        // Thick outlines are fitted inside the thumbnail; PDF values remain unchanged.
        let previewWidth = min(style.lineWidth, 8)
        let inset = max(5, previewWidth / 2 + 2)
        let rect = bounds.insetBy(dx: inset, dy: inset)
        let path: NSBezierPath
        switch tool {
        case .rectangle: path = NSBezierPath(rect: rect)
        case .ellipse: path = NSBezierPath(ovalIn: rect)
        case .line, .arrow, .doubleArrow:
            path = NSBezierPath()
            let start = NSPoint(x: rect.minX, y: rect.minY + 2)
            let end = NSPoint(x: rect.maxX, y: rect.maxY - 2)
            path.move(to: start); path.line(to: end)
            if tool == .arrow || tool == .doubleArrow { addArrow(to: path, tip: end, from: start) }
            if tool == .doubleArrow { addArrow(to: path, tip: start, from: end) }
        }
        if tool.supportsFill, let fill = style.fillColor { fill.setFill(); path.fill() }
        style.strokeColor.setStroke()
        path.lineWidth = previewWidth
        path.lineJoinStyle = .round
        path.lineCapStyle = .round
        path.stroke()
    }

    private func addArrow(to path: NSBezierPath, tip: NSPoint, from tail: NSPoint) {
        let angle = atan2(tip.y - tail.y, tip.x - tail.x)
        let length: CGFloat = 7
        for offset: CGFloat in [-0.55, 0.55] {
            path.move(to: tip)
            path.line(to: NSPoint(x: tip.x - length * cos(angle + offset), y: tip.y - length * sin(angle + offset)))
        }
    }
}
