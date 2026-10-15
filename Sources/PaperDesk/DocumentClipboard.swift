import AppKit

/// Typed system pasteboard entries allow independent PDF tabs to exchange objects.
/// No application-private stale cache survives a later copy in another app.
@MainActor final class DocumentClipboard {
    static let shared = DocumentClipboard(pasteboard: .general)
    static let pagesType = NSPasteboard.PasteboardType("cn.nankong.pdfmaster.pages.pdf")
    static let annotationType = NSPasteboard.PasteboardType("cn.nankong.pdfmaster.annotation.pdf")
    let pasteboard: NSPasteboard
    init(pasteboard: NSPasteboard) { self.pasteboard = pasteboard }
    var pages: Data? { pasteboard.data(forType: Self.pagesType) }
    var annotation: Data? { pasteboard.data(forType: Self.annotationType) }
    var hasPages: Bool { pasteboard.availableType(from: [Self.pagesType]) != nil }
    var hasAnnotation: Bool { pasteboard.availableType(from: [Self.annotationType]) != nil }
    @discardableResult func write(_ data: Data, type: NSPasteboard.PasteboardType) -> Bool {
        let item = NSPasteboardItem()
        guard item.setData(data, forType: type) else { return false }
        if type == Self.pagesType { item.setData(data, forType: .pdf) }
        pasteboard.clearContents()
        return pasteboard.writeObjects([item])
    }
}

enum ClipboardCommand {
    case copy, cut, paste, selectAll
    init?(_ action: Selector?) {
        switch action {
        case #selector(NSText.copy(_:)): self = .copy
        case #selector(NSText.cut(_:)): self = .cut
        case #selector(NSText.paste(_:)): self = .paste
        case #selector(NSText.selectAll(_:)): self = .selectAll
        default: return nil
        }
    }
}

/// Text editors keep their own history, including in secondary windows.
@MainActor enum TextHistoryRouting {
    static func editor(in window: NSWindow?) -> NSTextView? { window?.firstResponder as? NSTextView }
    static func canPerform(in window: NSWindow?, redo: Bool) -> Bool? {
        guard let editor = editor(in: window) else { return nil }
        return redo ? editor.undoManager?.canRedo == true : editor.undoManager?.canUndo == true
    }
    @discardableResult static func perform(in window: NSWindow?, redo: Bool) -> Bool {
        guard let editor = editor(in: window) else { return false }
        if redo { editor.undoManager?.redo() } else { editor.undoManager?.undo() }
        return true // Never fall through to document history while editing text.
    }
}
