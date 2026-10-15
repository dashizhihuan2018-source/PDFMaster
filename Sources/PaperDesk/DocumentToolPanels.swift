import AppKit
import PaperCore
import PDFKit

struct EncryptionCredentials {
    let userPassword: String
    let ownerPassword: String
}

struct NoteConfiguration {
    let contents: String
    let author: String?
    let color: NSColor
}

struct AnnotationConfiguration {
    let contents: String?
    let style: AnnotationStyle
}

struct AnnotationEditConfiguration {
    let contents: String?
    let bounds: CGRect
    let color: NSColor
}

@MainActor
enum DocumentToolPanels {
    static func passwordForOpening() -> String? {
        let field = NSSecureTextField(string: "")
        field.placeholderString = "PDF 密码"
        field.setAccessibilityLabel("PDF 密码")
        field.frame = NSRect(x: 0, y: 0, width: 320, height: 24)
        let alert = NSAlert()
        alert.messageText = "打开加密 PDF"
        alert.informativeText = "密码仅用于本次打开，不会被保存。"
        alert.accessoryView = field
        alert.addButton(withTitle: "解锁")
        alert.addButton(withTitle: "取消")
        return alert.runModal() == .alertFirstButtonReturn ? field.stringValue : nil
    }

    static func encryptionCredentials() -> EncryptionCredentials? {
        let user = NSSecureTextField(string: "")
        let userConfirmation = NSSecureTextField(string: "")
        let owner = NSSecureTextField(string: "")
        let ownerConfirmation = NSSecureTextField(string: "")
        user.placeholderString = "用户密码"
        userConfirmation.placeholderString = "确认用户密码"
        owner.placeholderString = "所有者密码"
        ownerConfirmation.placeholderString = "确认所有者密码"
        [user, userConfirmation, owner, ownerConfirmation].forEach { $0.setAccessibilityLabel($0.placeholderString ?? "密码") }
        let stack = verticalStack([user, userConfirmation, owner, ownerConfirmation], width: 340)
        let alert = NSAlert()
        alert.messageText = "存储密码保护副本"
        alert.informativeText = "请设置非空的用户密码和所有者密码。此功能用于新建或未加密来源；既有加密 PDF 当前会拒绝重新加密导出。"
        alert.accessoryView = stack
        alert.addButton(withTitle: "继续")
        alert.addButton(withTitle: "取消")
        guard alert.runModal() == .alertFirstButtonReturn else { return nil }
        guard !user.stringValue.isEmpty, !owner.stringValue.isEmpty,
              user.stringValue == userConfirmation.stringValue,
              owner.stringValue == ownerConfirmation.stringValue else {
            showValidationError("密码不能为空，且两次输入必须一致。")
            return nil
        }
        return EncryptionCredentials(userPassword: user.stringValue, ownerPassword: owner.stringValue)
    }

    static func noteConfiguration() -> NoteConfiguration? {
        let contents = NSTextField(string: "")
        contents.placeholderString = "批注内容"
        let author = NSTextField(string: "")
        author.placeholderString = "作者（可选）"
        let color = NSColorWell(); color.color = .systemYellow
        let stack = verticalStack([contents, author, labeled("颜色", control: color)], width: 340)
        guard runAlert(title: "便筺批注", information: "确定后在页面上拖动放置区域。", view: stack) else { return nil }
        guard !contents.stringValue.isEmpty else { showValidationError("批注内容不能为空。"); return nil }
        return NoteConfiguration(contents: contents.stringValue, author: author.stringValue.isEmpty ? nil : author.stringValue, color: color.color)
    }

    static func annotationConfiguration(includesContents: Bool = false) -> AnnotationConfiguration? {
        let contents = NSTextField(string: "")
        contents.placeholderString = "批注内容（可选）"
        let stroke = NSColorWell(); stroke.color = .systemRed
        let fillToggle = NSButton(checkboxWithTitle: "使用填充色", target: nil, action: nil)
        let fill = NSColorWell(); fill.color = NSColor.systemYellow.withAlphaComponent(0.25)
        let width = NSTextField(string: "2")
        width.placeholderString = "线宽（点）"
        var views: [NSView] = []
        if includesContents { views.append(contents) }
        views += [labeled("描边色", control: stroke), fillToggle, labeled("填充色", control: fill), width]
        let stack = verticalStack(views, width: 340)
        guard runAlert(title: "批注样式", information: "确定后在页面上拖动放置。", view: stack) else { return nil }
        do {
            let style = try ToolInputValidation.style(stroke: stroke.color, fill: fillToggle.state == .on ? fill.color : nil, width: width.stringValue)
            return AnnotationConfiguration(contents: contents.stringValue.isEmpty ? nil : contents.stringValue, style: style)
        } catch {
            showValidationError("线宽必须是大于 0 的有限数字。")
            return nil
        }
    }

    static func editConfiguration(annotation: PDFAnnotation) -> AnnotationEditConfiguration? {
        let contents = NSTextField(string: annotation.contents ?? "")
        contents.placeholderString = "批注内容（可留空）"
        let bounds = annotation.bounds
        let x = NSTextField(string: String(format: "%.2f", bounds.minX))
        let y = NSTextField(string: String(format: "%.2f", bounds.minY))
        let width = NSTextField(string: String(format: "%.2f", bounds.width))
        let height = NSTextField(string: String(format: "%.2f", bounds.height))
        let color = NSColorWell(); color.color = annotation.color
        let geometry = horizontalStack([
            labeled("X", control: x), labeled("Y", control: y),
            labeled("W", control: width), labeled("H", control: height)
        ])
        let stack = verticalStack([contents, geometry, labeled("颜色", control: color)], width: 440)
        guard runAlert(title: "编辑选中批注", information: "位置和尺寸使用 PDF 页面点坐标。", view: stack) else { return nil }
        do {
            return AnnotationEditConfiguration(
                contents: contents.stringValue,
                bounds: try ToolInputValidation.rectangle(x: x.stringValue, y: y.stringValue, width: width.stringValue, height: height.stringValue),
                color: color.color
            )
        } catch {
            showValidationError("坐标和尺寸必须为有限数字，宽高必须大于 0。")
            return nil
        }
    }

    static func cropRectangle(mediaBox: CGRect) -> CGRect? {
        let x = NSTextField(string: String(format: "%.2f", mediaBox.minX))
        let y = NSTextField(string: String(format: "%.2f", mediaBox.minY))
        let width = NSTextField(string: String(format: "%.2f", mediaBox.width))
        let height = NSTextField(string: String(format: "%.2f", mediaBox.height))
        let stack = horizontalStack([
            labeled("X", control: x), labeled("Y", control: y),
            labeled("W", control: width), labeled("H", control: height)
        ])
        guard runAlert(
            title: "裁剪当前页",
            information: "请输入 PDF 页面点坐标。裁剪只改变可见区域，不会删除隐藏内容，也不是安全涂黑。",
            view: stack
        ) else { return nil }
        do {
            return try ToolInputValidation.rectangle(x: x.stringValue, y: y.stringValue, width: width.stringValue, height: height.stringValue, within: mediaBox)
        } catch {
            showValidationError("裁剪矩形必须完全位于 MediaBox 内。")
            return nil
        }
    }

    private static func runAlert(title: String, information: String, view: NSView) -> Bool {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = information
        alert.accessoryView = view
        alert.addButton(withTitle: "确定")
        alert.addButton(withTitle: "取消")
        return alert.runModal() == .alertFirstButtonReturn
    }

    private static func showValidationError(_ text: String) {
        let alert = NSAlert(); alert.alertStyle = .warning; alert.messageText = "输入无效"; alert.informativeText = text; alert.runModal()
    }

    private static func verticalStack(_ views: [NSView], width: CGFloat) -> NSStackView {
        let stack = NSStackView(views: views)
        stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 8
        stack.frame = NSRect(x: 0, y: 0, width: width, height: CGFloat(views.count) * 32)
        views.forEach { $0.widthAnchor.constraint(lessThanOrEqualToConstant: width).isActive = true }
        return stack
    }

    private static func horizontalStack(_ views: [NSView]) -> NSStackView {
        let stack = NSStackView(views: views)
        stack.orientation = .horizontal; stack.alignment = .centerY; stack.spacing = 8
        stack.frame = NSRect(x: 0, y: 0, width: 440, height: 34)
        return stack
    }

    private static func labeled(_ title: String, control: NSView) -> NSView {
        let label = NSTextField(labelWithString: title)
        label.setContentHuggingPriority(.required, for: .horizontal)
        let stack = NSStackView(views: [label, control])
        stack.orientation = .horizontal; stack.alignment = .centerY; stack.spacing = 6
        return stack
    }
}
