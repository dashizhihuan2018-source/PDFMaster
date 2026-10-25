import Foundation
import CoreGraphics
import CoreText

/// REA-derived boundary: PDF Expert RDPDFKitCore 0x24a00c draws edited
/// attributed text into a temporary PDF, then imports its streams/resources.
/// This uses system APIs only, never loads or links PDF Expert's frameworks.
/// Single horizontal span only; not a reproduction of its paragraph engine.
enum NativeContentTextPDF {
    static func render(_ text: String, span: ContentSpan, page: ContentPage, style: ContentTextStyle? = nil) throws -> (Data, String, CGRect) {
        guard !text.isEmpty, text.count <= 4096,
              !text.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
              span.origin.count == 2, span.origin.allSatisfy(\.isFinite),
              span.size.isFinite, span.size > 0, span.size <= 4096 else { throw OrganizeError.invalidInput }
        if let style { guard style.isValid else { throw OrganizeError.invalidInput } }
        let retainsOriginalFont = style == nil || style?.retainsOriginalFont == true
        if style?.retainsOriginalFont == true, style?.fontName != span.font { throw OrganizeError.invalidInput }
        let supplied = retainsOriginalFont ? (span.font.components(separatedBy: "+").last ?? span.font) : style!.fontName
        let available = CTFontManagerCopyAvailablePostScriptNames() as? [String] ?? []
        let aliases = supplied.localizedCaseInsensitiveContains("FangSong") ? ["STFangsong", "FangSong", "SongtiSC-Regular"] : []
        let name = ([supplied] + aliases).compactMap { candidate in available.first { $0.caseInsensitiveCompare(candidate) == .orderedSame } }.first ?? supplied
        let font = CTFontCreateWithName(name as CFString, style?.size ?? span.size, nil)
        // Explicit user choices must exist; never silently substitute an unknown
        // font. Original embedded/subset fonts retain the existing fallback path.
        if !retainsOriginalFont, !available.contains(where: { $0.caseInsensitiveCompare(name) == .orderedSame }),
           (CTFontCopyPostScriptName(font) as String).caseInsensitiveCompare(name) != .orderedSame {
            throw OrganizeError.processFailed("所选字体在本机不可用；草稿已保留")
        }
        let rgb = style?.color ?? span.color
        guard let color = CGColor(colorSpace: CGColorSpaceCreateDeviceRGB(), components: [
            CGFloat((rgb >> 16) & 255) / 255, CGFloat((rgb >> 8) & 255) / 255,
            CGFloat(rgb & 255) / 255, 1
        ]) else { throw OrganizeError.invalidInput }
        let attributed = NSAttributedString(string: text, attributes: [
            NSAttributedString.Key(kCTFontAttributeName as String): font,
            NSAttributedString.Key(kCTForegroundColorAttributeName as String): color
        ])
        let line = CTLineCreateWithAttributedString(attributed)
        var names = Set<String>()
        for run in CTLineGetGlyphRuns(line) as? [CTRun] ?? [] {
            let count = CTRunGetGlyphCount(run)
            var glyphs = [CGGlyph](repeating: 0, count: count)
            CTRunGetGlyphs(run, CFRange(location: 0, length: count), &glyphs)
            guard !glyphs.contains(0) else { throw OrganizeError.processFailed("Unsupported replacement glyphs in native font layout") }
            let attributes = CTRunGetAttributes(run) as NSDictionary
            if let runFont = attributes[kCTFontAttributeName] {
                names.insert(CTFontCopyPostScriptName(runFont as! CTFont) as String)
            }
        }
        guard !names.isEmpty else { throw OrganizeError.invalidInput }
        let data = NSMutableData()
        var media = CGRect(x: 0, y: 0, width: page.width, height: page.height)
        guard let consumer = CGDataConsumer(data: data),
              let context = CGContext(consumer: consumer, mediaBox: &media, [kCGPDFContextCreator: "PDF Master native content text"] as CFDictionary) else { throw OrganizeError.invalidInput }
        context.beginPDFPage(nil)
        context.textMatrix = .identity
        context.textPosition = CGPoint(x: span.origin[0], y: page.height - span.origin[1])
        CTLineDraw(line, context)
        context.endPDFPage(); context.closePDF()
        let bounds = CTLineGetBoundsWithOptions(line,[.useGlyphPathBounds]).offsetBy(dx:span.origin[0],dy:page.height-span.origin[1])
        return (data as Data, names.sorted().joined(separator: ", "), bounds)
    }
}
