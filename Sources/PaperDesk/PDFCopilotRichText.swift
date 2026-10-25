import AppKit
import PaperCore

/// Native, execution-free Markdown. Typography follows the shipped RDCopilotUI
/// MarkdownParametersFactory (arm64 0x12650, 0x129c0...0x12a1c), not HTML/CSS.
@MainActor enum PDFCopilotRichText {
    private static let codeAttribute = NSAttributedString.Key("PDFCopilotCode")
    private static func paragraph() -> NSMutableParagraphStyle {
        let p = NSMutableParagraphStyle()
        p.lineHeightMultiple = 1.16; p.paragraphSpacing = 12
        return p
    }
    static func renderMarkdown(_ source: String, citedPages: Set<Int>) -> NSAttributedString {
        guard !source.isEmpty else { return NSAttributedString(string: "") }
        guard let parsed = try? AttributedString(markdown: source, options: .init(interpretedSyntax: .full)) else {
            return NSAttributedString(string: source, attributes: [.font: NSFont.systemFont(ofSize: 15), .foregroundColor: NSColor.labelColor, .paragraphStyle: paragraph()])
        }
        let output = NSMutableAttributedString(string: "")
        var previousBlock: Int?; var lastAttributes: [NSAttributedString.Key: Any] = [:]
        var emittedListItems = Set<Int>()
        for run in parsed.runs {
            let components = run.presentationIntent?.components ?? []
            let block = components.first?.identity
            let newBlock = output.length == 0 || block != previousBlock
            let style = paragraph()
            var font = NSFont.systemFont(ofSize: 15)
            var color = NSColor.labelColor
            var isCode = false; var heading = false; var ordinal: Int?; var itemID: Int?
            var ordered = false; var depth = 0
            for component in components {
                switch component.kind {
                case .header(let level):
                    font = .systemFont(ofSize: level == 1 ? 17 : level == 2 ? 16 : 15, weight: .semibold); heading = true
                case .codeBlock: isCode = true
                case .blockQuote: style.headIndent += 16; style.firstLineHeadIndent += 16; color = .secondaryLabelColor
                case .orderedList: if depth == 0 { ordered = true }; depth += 1
                case .unorderedList: depth += 1
                case .listItem(let value):
                    if itemID == nil { ordinal = value; itemID = component.identity }
                default: break
                }
            }
            if depth > 0 {
                style.firstLineHeadIndent += CGFloat(max(0, depth - 1)) * 30
                style.headIndent += CGFloat(depth) * 30; style.paragraphSpacing = 6
                style.tabStops = [NSTextTab(textAlignment: .left, location: style.headIndent)]
            }
            if heading { style.paragraphSpacingBefore = 4 }
            let inline = run.inlinePresentationIntent
            isCode = isCode || inline?.contains(.code) == true
            if isCode { font = .monospacedSystemFont(ofSize: 14, weight: .regular) }
            else {
                if inline?.contains(.stronglyEmphasized) == true { font = NSFontManager.shared.convert(font, toHaveTrait: .boldFontMask) }
                if inline?.contains(.emphasized) == true { font = NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask) }
            }
            var attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color, .paragraphStyle: style]
            if isCode { attributes[codeAttribute] = true; attributes[.backgroundColor] = NSColor.quaternaryLabelColor }
            if inline?.contains(.strikethrough) == true { attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
            if newBlock, output.length > 0 { output.append(NSAttributedString(string: "\n", attributes: lastAttributes)) }
            if newBlock, let ordinal, let itemID, !emittedListItems.contains(itemID) {
                output.append(NSAttributedString(string: ordered ? "\(ordinal).\t" : "•\t", attributes: attributes))
                emittedListItems.insert(itemID)
            }
            // Foundation marks a soft break as a space. Preserve the AI's line
            // breaks (e.g. a bold section label followed by its paragraph).
            let text = inline?.contains(.softBreak) == true ? "\n" : String(parsed[run.range].characters)
            output.append(NSAttributedString(string: text, attributes: attributes))
            previousBlock = block; lastAttributes = attributes
        }
        // Only document-validated citations are actionable. Markdown links and
        // images never become URL actions, attachments, HTML, or remote loads.
        let regex = try! NSRegularExpression(pattern: #"\[p\.(\d+)\]"#)
        let content = output.string as NSString
        for match in regex.matches(in: output.string, range: NSRange(location: 0, length: output.length)) {
            guard let page = Int(content.substring(with: match.range(at: 1))), citedPages.contains(page),
                  output.attribute(codeAttribute, at: match.range.location, effectiveRange: nil) == nil,
                  let url = URL(string: "pdfmaster-citation://page/\(page)") else { continue }
            output.addAttributes([.link: url, .foregroundColor: NSColor.systemIndigo, .font: NSFont.systemFont(ofSize: 11, weight: .semibold), .baselineOffset: 2, .backgroundColor: NSColor.systemIndigo.withAlphaComponent(0.12)], range: match.range)
        }
        return output
    }
    static func conversation(_ messages: [DMXMessage], citedPages: Set<Int>) -> NSAttributedString {
        let output = NSMutableAttributedString(string: "")
        for message in messages {
            if output.length > 0 { output.append(NSAttributedString(string: "\n\n")) }
            let role = NSAttributedString(string: message.role == "user" ? "你：\n" : "PDF Copilot：\n", attributes: [.font: NSFont.systemFont(ofSize: 12, weight: .semibold), .foregroundColor: NSColor.secondaryLabelColor, .paragraphStyle: paragraph()])
            output.append(role)
            if message.role == "user" {
                output.append(NSAttributedString(string: message.content, attributes: [.font: NSFont.systemFont(ofSize: 15), .foregroundColor: NSColor.labelColor, .paragraphStyle: paragraph()]))
            } else { output.append(renderMarkdown(message.content, citedPages: citedPages)) }
        }
        return output
    }
}
