import AppKit
import PDFKit

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
        if type == Self.pagesType {
            guard let document = PDFDocument(data: data), let image = pageImage(document),
                  let png = image.representation(using: .png, properties: [:]),
                  let tiff = image.representation(using: .tiff, properties: [.compressionMethod: NSBitmapImageRep.TIFFCompression.lzw.rawValue]) else { return false }
            item.setData(data, forType: .pdf)
            item.setData(png, forType: .png)
            item.setData(tiff, forType: .tiff)
        }
        pasteboard.clearContents()
        return pasteboard.writeObjects([item])
    }

    /// External receivers get a lossless, bounded image; internal tabs retain every PDF page.
    /// Multiple selected pages are stacked in order, rather than silently dropping later pages.
    private func pageImage(_ document: PDFDocument) -> NSBitmapImageRep? {
        let pages = (0..<document.pageCount).compactMap { document.page(at: $0) }
        guard !pages.isEmpty else { return nil }
        let sizes = pages.map { page -> CGSize in
            let box = page.bounds(for: .cropBox)
            return page.rotation % 180 == 0 ? box.size : CGSize(width: box.height, height: box.width)
        }
        guard sizes.allSatisfy({ $0.width.isFinite && $0.height.isFinite && $0.width > 0 && $0.height > 0 }) else { return nil }
        let width = sizes.map(\.width).max()!, height = sizes.reduce(0) { $0 + $1.height } + CGFloat(pages.count - 1) * 12
        let scale = min(3, 8192 / max(width, height), sqrt(16_777_216 / (width * height)))
        let pixelWidth = max(1, Int(floor(width * scale))), pixelHeight = max(1, Int(floor(height * scale)))
        guard let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixelWidth, pixelsHigh: pixelHeight,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
              let context = NSGraphicsContext(bitmapImageRep: bitmap) else { return nil }
        NSGraphicsContext.saveGraphicsState(); defer { NSGraphicsContext.restoreGraphicsState() }
        NSGraphicsContext.current = context; context.imageInterpolation = .high
        NSColor.white.setFill(); CGRect(x: 0, y: 0, width: pixelWidth, height: pixelHeight).fill()
        var top = CGFloat(pixelHeight)
        for (page, size) in zip(pages, sizes) {
            let rendered = CGSize(width: size.width * scale, height: size.height * scale)
            top -= rendered.height
            page.thumbnail(of: rendered, for: .cropBox).draw(in: CGRect(x: (CGFloat(pixelWidth) - rendered.width) / 2,
                y: top, width: rendered.width, height: rendered.height), from: .zero, operation: .sourceOver, fraction: 1)
            top -= 12 * scale
        }
        context.flushGraphics()
        return bitmap
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
