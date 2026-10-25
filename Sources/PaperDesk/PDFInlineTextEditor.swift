import AppKit
import PDFKit

/// An uncommitted, page-local draft. NSTextView owns IME, selection, clipboard
/// and typing undo; the document receives exactly one transaction on commit.
@MainActor final class PDFInlineTextSession: NSObject, NSTextViewDelegate, NSComboBoxDelegate {
    weak var pdfView: InkPDFView?
    let page: PDFPage
    let textView = PDFDraftTextView()
    let container = PDFTextEditorContainer()
    private let toolbar = NSStackView()
    private weak var toolbarHost: NSView?
    private let scroll = NSScrollView()
    private let fontField = NSComboBox()
    private let sizeField = NSComboBox()
    private let boldButton = NSButton(title: "B", target: nil, action: nil)
    private let colorButton = NSColorWell()
    private let commitHandler: (String, CGRect, NSFont, NSColor) throws -> Void
    private let failureHandler: (Error) -> Void
    private var monitor: Any?
    private var finishing = false
    private var updating = false
    private var baseFont: NSFont
    private var textColor: NSColor
    private(set) var pageBounds: CGRect
    private var originalAnnotation: PDFAnnotation?
    var editedAnnotation: PDFAnnotation? { originalAnnotation }
    private var originalDisplay = true
    private let initialText: String
    private let initialBounds: CGRect
    private let initialFont: NSFont
    private let initialColor: NSColor
    private let isExisting: Bool
    private let stylesEnabled: Bool
    private let allowsFractionalFontSize: Bool
    var hasDraftChanges: Bool {
        textView.string != initialText || pageBounds != initialBounds || baseFont != initialFont || textColor != initialColor
    }
    deinit { if let monitor { MainActor.assumeIsolated { NSEvent.removeMonitor(monitor) } } }

    init(pdfView: InkPDFView, page: PDFPage, bounds: CGRect, text: String,
         font: NSFont, color: NSColor, annotation: PDFAnnotation? = nil, stylesEnabled: Bool = true, allowsEmptyCommit: Bool = false,
         allowsFractionalFontSize: Bool = false,
         commit: @escaping (String, CGRect, NSFont, NSColor) throws -> Void,
         failure: @escaping (Error) -> Void) {
        self.pdfView = pdfView; self.page = page; pageBounds = bounds
        initialText = text; initialBounds = bounds; initialFont = font; initialColor = color
        baseFont = font; textColor = color; originalAnnotation = annotation; isExisting = annotation != nil || allowsEmptyCommit
        self.stylesEnabled = stylesEnabled
        self.allowsFractionalFontSize = allowsFractionalFontSize
        commitHandler = commit; failureHandler = failure
        super.init()
        container.wantsLayer = true; container.layer?.borderColor = NSColor.controlAccentColor.cgColor
        container.layer?.borderWidth = 1; container.layer?.cornerRadius = 3
        // The PDF is white even when the application chrome is dark. Semantic
        // textBackgroundColor would produce black text on a dark input surface.
        container.layer?.backgroundColor = NSColor.white.cgColor
        container.layer?.zPosition = 2000
        textView.isRichText = false; textView.importsGraphics = false
        textView.backgroundColor = .white
        textView.allowsUndo = true; textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.textContainerInset = NSSize(width: 2, height: 2)
        textView.textContainer?.lineFragmentPadding = 0
        textView.isHorizontallyResizable = false; textView.isVerticallyResizable = true
        textView.autoresizingMask = [.width]; textView.string = text
        textView.setAccessibilityLabel("页面内文字输入")
        textView.delegate = self
        textView.onCancel = { [weak self] in _ = self?.finish(commit: false) }
        textView.onCommit = { [weak self] in _ = self?.finish() }
        scroll.borderType = .noBorder; scroll.hasVerticalScroller = true
        scroll.drawsBackground = false; scroll.documentView = textView
        container.addSubview(scroll)
        let bar = toolbar; bar.spacing = 5; bar.orientation = .horizontal
        bar.alignment = .centerY
        bar.wantsLayer = true; bar.layer?.backgroundColor = NSColor.controlBackgroundColor.cgColor
        bar.layer?.cornerRadius = 4; bar.layer?.zPosition = 2001
        fontField.addItems(withObjectValues: NSFontManager.shared.availableFontFamilies.sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending })
        fontField.stringValue = font.familyName ?? font.fontName
        fontField.setAccessibilityLabel("文字字体")
        fontField.toolTip = "选择或输入本机字体族；保留当前字号与字重"
        sizeField.addItems(withObjectValues: ["1", "8", "10", "12", "14", "16", "18", "24", "32", "48", "72", "128", "256", "512"])
        sizeField.stringValue = String(format: "%g", font.pointSize)
        sizeField.setAccessibilityLabel("文字字号")
        sizeField.toolTip = allowsFractionalFontSize ? "输入任意 1–512 点字号，支持小数" : "输入任意 1–512 点整数字号（PDF 批注格式）"
        for field in [fontField, sizeField] {
            field.isEditable = true; field.completes = true; field.controlSize = .small
            field.font = .systemFont(ofSize: 11); field.delegate = self
            field.target = self; field.action = #selector(changeStyle)
            field.isEnabled = stylesEnabled
        }
        fontField.widthAnchor.constraint(equalToConstant: 136).isActive = true
        sizeField.widthAnchor.constraint(equalToConstant: 65).isActive = true
        boldButton.bezelStyle = .inline; boldButton.setButtonType(.toggle)
        boldButton.state = NSFontManager.shared.traits(of: font).contains(.boldFontMask) ? .on : .off
        boldButton.target = self; boldButton.action = #selector(changeStyle); boldButton.setAccessibilityLabel("文字加粗")
        colorButton.color = color; colorButton.target = self; colorButton.action = #selector(changeStyle)
        colorButton.isContinuous = true
        colorButton.setAccessibilityLabel("文字颜色"); colorButton.frame.size = NSSize(width: 30, height: 22)
        sizeField.isEnabled = stylesEnabled; boldButton.isEnabled = stylesEnabled; colorButton.isEnabled = stylesEnabled
        let done = NSButton(title: "完成", target: self, action: #selector(commitAction))
        done.bezelStyle = .inline; done.setAccessibilityLabel("完成文字输入")
        let cancel = NSButton(title: "取消", target: self, action: #selector(cancelAction))
        cancel.bezelStyle = .inline; cancel.setAccessibilityLabel("取消文字输入")
        [fontField, sizeField, boldButton, colorButton, done, cancel].forEach { bar.addArrangedSubview($0) }
        pdfView.addSubview(container, positioned: .above, relativeTo: nil)
        // Text styling belongs to the workbench's fixed top row. In isolated
        // PDF views there is deliberately no floating fallback over the page.
        if let host = pdfView.textStyleToolbarHost {
            toolbarHost = host; host.isHidden = false
            bar.translatesAutoresizingMaskIntoConstraints = false
            host.addSubview(bar)
            NSLayoutConstraint.activate([
                bar.leadingAnchor.constraint(equalTo: host.leadingAnchor),
                bar.trailingAnchor.constraint(equalTo: host.trailingAnchor),
                bar.topAnchor.constraint(equalTo: host.topAnchor),
                bar.bottomAnchor.constraint(equalTo: host.bottomAnchor)
            ])
            pdfView.onTextStyleToolbarVisibilityChange?(true)
        }
        if let annotation { originalDisplay = annotation.shouldDisplay; annotation.shouldDisplay = false; pdfView.needsDisplay = true }
        reposition(); textView.setSelectedRange(NSRange(location: (text as NSString).length, length: 0))
        pdfView.refreshCommittedInkRendering()
        pdfView.window?.makeFirstResponder(textView)
        monitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown) { [weak self] event in
            // The local AppKit monitor executes synchronously on the main thread.
                guard let self, !self.finishing, event.window === self.pdfView?.window,
                      let host = self.pdfView else { return event }
                let point = self.container.convert(event.locationInWindow, from: nil)
                if self.container.bounds.contains(point) || self.isStyleInteraction(event) { return event }
                guard self.finish() else { host.window?.makeFirstResponder(self.textView); return nil }
                return event
        }
    }

    func reposition() {
        guard !updating, let host = pdfView, host.document?.index(for: page) != NSNotFound else { return }
        updating = true; defer { updating = false }
        let rect = host.convert(pageBounds, from: page)
        let scale = max(0.01, host.scaleFactor)
        textView.font = NSFontManager.shared.convert(baseFont, toSize: baseFont.pointSize * scale)
        textView.textColor = textColor
        // PDFKit's FreeText appearance uses a taller fallback-CJK line grid than
        // NSTextView's system default. Reserve the same conservative grid in the
        // draft so a successful commit cannot clip the final line of Chinese.
        let paragraph = NSMutableParagraphStyle()
        paragraph.minimumLineHeight = textView.font!.pointSize * 1.5
        textView.defaultParagraphStyle = paragraph
        textView.typingAttributes = [.font: textView.font!, .foregroundColor: textColor, .paragraphStyle: paragraph]
        textView.textStorage?.addAttribute(.paragraphStyle, value: paragraph,
            range: NSRange(location: 0, length: (textView.string as NSString).length))
        container.frame = rect
        scroll.frame = CGRect(origin: .zero, size: rect.size)
        textView.frame.size.width = rect.width
        textView.textContainer?.containerSize = NSSize(width: max(1, rect.width - 4), height: .greatestFiniteMagnitude)
        textView.layoutManager?.ensureLayout(for: textView.textContainer!)
        let height = textView.layoutManager?.usedRect(for: textView.textContainer!).height ?? 0
        textView.frame.size.height = max(rect.height, height + 4)
    }

    private func isStyleInteraction(_ event: NSEvent) -> Bool {
        if let host = toolbarHost, !host.isHidden,
           host.bounds.contains(host.convert(event.locationInWindow, from: nil)) { return true }
        // An active combo's native field editor can be a window-owned view,
        // rather than a descendant of the combo. Do not treat its clicks as a
        // page click. Popup/colour-panel events have another window and are
        // already excluded by the monitor's same-window guard above.
        for field in [fontField, sizeField] {
            if let editor = field.currentEditor(), editor.window === event.window,
               editor.bounds.contains(editor.convert(event.locationInWindow, from: nil)) { return true }
        }
        return false
    }

    func textDidChange(_ notification: Notification) { growToFit() }
    func textView(_ textView: NSTextView, shouldChangeTextIn affectedCharRange: NSRange, replacementString: String?) -> Bool {
        guard let replacementString else { return true }
        let length = (textView.string as NSString).length
        guard affectedCharRange.location <= length, affectedCharRange.length <= length - affectedCharRange.location,
              length - affectedCharRange.length + replacementString.utf16.count <= 100_000,
              !replacementString.unicodeScalars.contains(where: {
                  CharacterSet.controlCharacters.contains($0) && $0 != "\n" && $0 != "\r" && $0 != "\t"
              }) else { NSSound.beep(); return false }
        return true
    }

    private func growToFit() {
        guard let host = pdfView, !updating else { return }
        textView.layoutManager?.ensureLayout(for: textView.textContainer!)
        let used = (textView.layoutManager?.usedRect(for: textView.textContainer!).height ?? 0) + 6
        let frame = host.convert(pageBounds, from: page)
        let crop = host.convert(page.bounds(for: .cropBox), from: page)
        let height = min(max(frame.height, used), max(1, frame.maxY - crop.minY))
        let grown = CGRect(x: frame.minX, y: frame.maxY - height, width: frame.width, height: height)
        pageBounds = host.convert(grown, to: page).intersection(page.bounds(for: .cropBox))
        reposition()
    }

    /// Preview complete valid values while editing either native combo. Partial
    /// or invalid input never silently resets the document's previous style.
    func controlTextDidChange(_ notification: Notification) { _ = applyStyle(reportFailure: false) }
    func comboBoxSelectionDidChange(_ notification: Notification) {
        _ = applyStyle(reportFailure: false)
    }
    @objc private func changeStyle() {
        if applyStyle(reportFailure: true) { pdfView?.window?.makeFirstResponder(textView) }
    }
    @discardableResult private func applyStyle(reportFailure: Bool) -> Bool {
        guard stylesEnabled else { return true }
        let familyName = (fontField.currentEditor()?.string ?? fontField.stringValue).trimmingCharacters(in: .whitespacesAndNewlines)
        let sizeText = (sizeField.currentEditor()?.string ?? sizeField.stringValue).trimmingCharacters(in: .whitespacesAndNewlines)
        guard let size = Double(sizeText), size.isFinite, (1...512).contains(size), allowsFractionalFontSize || size.rounded() == size else {
            if reportFailure { failureHandler(PDFInlineTextError.invalidFontSize) }
            return false
        }
        let manager = NSFontManager.shared
        var traits = manager.traits(of: baseFont)
        if boldButton.state == .on { traits.insert(.boldFontMask) } else { traits.remove(.boldFontMask) }
        let actualFamily = manager.availableFontFamilies.first { $0.caseInsensitiveCompare(familyName) == .orderedSame }
        let resolved: NSFont?
        if (baseFont.familyName ?? baseFont.fontName).caseInsensitiveCompare(familyName) == .orderedSame {
            // Native PDF fonts and the private system font may not appear in the
            // family picker. Resizing them must not silently choose a new face.
            let oldBold = manager.traits(of: baseFont).contains(.boldFontMask)
            let changed = oldBold == (boldButton.state == .on) ? baseFont : manager.convert(baseFont, toHaveTrait: boldButton.state == .on ? .boldFontMask : .unboldFontMask)
            resolved = manager.convert(changed, toSize: size)
        }
        else if let actualFamily { resolved = manager.font(withFamily: actualFamily, traits: traits, weight: 5, size: size) }
        else { resolved = NSFont(name: familyName, size: size).map { manager.convert($0, toHaveTrait: boldButton.state == .on ? .boldFontMask : .unboldFontMask) } }
        guard let font = resolved else {
            if reportFailure { failureHandler(PDFInlineTextError.invalidFontFamily) }
            return false
        }
        guard colorButton.color.usingColorSpace(.sRGB) != nil else { return false }
        baseFont = font; textColor = colorButton.color
        reposition(); growToFit()
        return true
    }
    @objc private func commitAction() { _ = finish() }
    @objc private func cancelAction() { _ = finish(commit: false) }

    @discardableResult func finish(commit: Bool = true) -> Bool {
        guard !finishing else { return false }
        if commit, !applyStyle(reportFailure: true) { return false }
        if commit, textView.hasMarkedText() { textView.unmarkText() }
        finishing = true
        if commit, (!isExisting || hasDraftChanges), !textView.string.isEmpty || isExisting {
            do {
                textView.layoutManager?.ensureLayout(for: textView.textContainer!)
                let used = textView.layoutManager?.usedRect(for: textView.textContainer!).height ?? 0
                if used + 3 > scroll.bounds.height { throw PDFInlineTextError.pageOverflow }
                // Restore display before copying authoritative annotations. Only the
                // presentation object was hidden; no transient flags enter the PDF.
                originalAnnotation?.shouldDisplay = originalDisplay
                try commitHandler(textView.string, pageBounds, baseFont, textColor)
            } catch {
                originalAnnotation?.shouldDisplay = false
                finishing = false; failureHandler(error); return false
            }
        }
        originalAnnotation?.shouldDisplay = originalDisplay
        if let monitor { NSEvent.removeMonitor(monitor); self.monitor = nil }
        colorButton.deactivate()
        container.removeFromSuperview()
        toolbar.removeFromSuperview()
        let host = pdfView
        if let toolbarHost { toolbarHost.isHidden = true; host?.onTextStyleToolbarVisibilityChange?(false) }
        host?.inlineTextSession = nil; host?.refreshCommittedInkRendering(); host?.needsDisplay = true
        if host?.window?.firstResponder === textView { host?.window?.makeFirstResponder(host) }
        return true
    }
}

enum PDFInlineTextError: LocalizedError {
    case pageOverflow, staleDocument, invalidFontFamily, invalidFontSize
    var errorDescription: String? {
        switch self {
        case .pageOverflow: return "文字超出页面，请缩小字号或减少内容；草稿已保留。"
        case .staleDocument: return "文档已变化，未写入过期草稿；请取消后重新编辑。"
        case .invalidFontFamily: return "未找到该本机字体，请选择列表中的字体；草稿已保留。"
        case .invalidFontSize: return "请输入 1–512 点之间的有效字号；新增文字批注仅支持整数字号。草稿已保留。"
        }
    }
}

@MainActor final class PDFDraftTextView: NSTextView {
    var onCancel: (() -> Void)?
    var onCommit: (() -> Void)?
    override func cancelOperation(_ sender: Any?) {
        // Escape dismisses a candidate composition first, not the entire draft.
        if hasMarkedText() { super.cancelOperation(sender) } else { onCancel?() }
    }
    override func keyDown(with event: NSEvent) {
        if event.keyCode == 36, event.modifierFlags.contains(.command), !hasMarkedText() { onCommit?(); return }
        super.keyDown(with: event)
    }
    override func paste(_ sender: Any?) { pasteAsPlainText(sender) }
}

@MainActor final class PDFTextEditorContainer: NSView {
    override var isFlipped: Bool { true }
}
