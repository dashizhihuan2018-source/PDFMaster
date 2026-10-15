import AppKit
import PDFKit

public enum AnnotationShape {
    case rectangle
    case ellipse
}

public enum LineEndings {
    case none
    case arrow
    case doubleArrow
}

public struct AnnotationStyle {
    /// Shared limits for editable PDF shape outlines, in page points.
    public static let supportedLineWidths: ClosedRange<CGFloat> = 0.25...32

    public var strokeColor: NSColor
    public var fillColor: NSColor?
    public var lineWidth: CGFloat

    public init(strokeColor: NSColor, fillColor: NSColor?, lineWidth: CGFloat) {
        self.strokeColor = strokeColor
        self.fillColor = fillColor
        self.lineWidth = lineWidth
    }

    /// PDF stores markup opacity separately from its RGB colour arrays.
    public init(annotation: PDFAnnotation) {
        let alpha = Self.nativeAlpha(annotation, key: "CA")
        let fillAlpha = Self.nativeAlpha(annotation, key: "ca") ?? alpha
        self.init(strokeColor: alpha.map { annotation.color.withAlphaComponent($0) } ?? annotation.color,
                  fillColor: annotation.interiorColor.map { color in fillAlpha.map { color.withAlphaComponent($0) } ?? color },
                  lineWidth: annotation.border?.lineWidth ?? 1)
    }

    private static func nativeAlpha(_ annotation: PDFAnnotation, key: String) -> CGFloat? {
        // Freshly edited colours take precedence over an imported appearance.
        let colorAlpha = key == "ca" ? annotation.interiorColor?.alphaComponent : annotation.color.alphaComponent
        if let colorAlpha, colorAlpha < 1 { return colorAlpha }
        guard annotation.hasAppearanceStream else { return nil }
        guard let page = annotation.page, let index = page.annotations.firstIndex(where: { $0 === annotation }),
              let pageDictionary = page.pageRef?.dictionary else { return nil }
        var array: CGPDFArrayRef?, dictionary: CGPDFDictionaryRef?
        guard CGPDFDictionaryGetArray(pageDictionary, "Annots", &array), let array,
              CGPDFArrayGetCount(array) == page.annotations.count,
              CGPDFArrayGetDictionary(array, index, &dictionary), let dictionary else { return nil }
        var direct: CGPDFReal = 1
        if CGPDFDictionaryGetNumber(dictionary, key, &direct), direct.isFinite, (0...1).contains(direct) { return direct }
        var appearance: CGPDFDictionaryRef?, stream: CGPDFStreamRef?, resources: CGPDFDictionaryRef?, states: CGPDFDictionaryRef?
        guard CGPDFDictionaryGetDictionary(dictionary, "AP", &appearance), let appearance,
              CGPDFDictionaryGetStream(appearance, "N", &stream), let stream,
              let streamDictionary = CGPDFStreamGetDictionary(stream),
              CGPDFDictionaryGetDictionary(streamDictionary, "Resources", &resources), let resources,
              CGPDFDictionaryGetDictionary(resources, "ExtGState", &states), let states,
              CGPDFDictionaryGetCount(states) <= 16 else { return nil }
        var values = Set<CGFloat>()
        CGPDFDictionaryApplyBlock(states, { _, object, _ in
            var state: CGPDFDictionaryRef?, number: CGPDFReal = 1
            if CGPDFObjectGetValue(object, .dictionary, &state), let state,
               CGPDFDictionaryGetNumber(state, key, &number), number.isFinite, (0...1).contains(number) {
                values.insert(number)
            }
            return true
        }, nil)
        // A complex appearance with several opacity values cannot be represented
        // by one slider. Do not guess which graphics state is active.
        return values.count == 1 ? values.first : nil
    }

    public static let `default` = AnnotationStyle(strokeColor: .black, fillColor: nil, lineWidth: 1)
}

public enum AnnotationSummaryFormat {
    case plainText
    case markdown
    case html
}
