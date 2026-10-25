import Foundation
import PDFKit

private final class FontDictionaryValues { var values: [CGPDFObjectRef] = [] }
private func collectFontValues(_ key: UnsafePointer<CChar>, _ value: CGPDFObjectRef, _ context: UnsafeMutableRawPointer?) {
    guard let context else { return }
    Unmanaged<FontDictionaryValues>.fromOpaque(context).takeUnretainedValue().values.append(value)
}

/// Repairs only positively identified malformed embedded CFF, in a private copy.
/// No network, source writes, system font changes, or rasterized page fallback.
enum PDFFontCompatibility {
    struct Result { let data: Data; let notice: String? }

    static func normalize(_ data: Data) -> Result {
        guard needsRepair(data) else { return Result(data: data, notice: nil) }
        let unresolved = "嵌入字体兼容修复未完成，部分文字可能显示异常；原文件未改动"
        do {
            let fixed = try NativeFontRepair.repair(data)
            guard let repaired = PDFDocument(data: fixed), !repaired.isEncrypted,
                  repaired.pageCount == PDFDocument(data: data)?.pageCount, !needsRepair(fixed) else {
                return Result(data: data, notice: unresolved)
            }
            return Result(data: fixed, notice: "已兼容修复异常嵌入字体 · 原文件未改动")
        } catch { return Result(data: data, notice: unresolved) }
    }

    static func needsRepair(_ data: Data) -> Bool {
        guard let provider = CGDataProvider(data: data as CFData), let pdf = CGPDFDocument(provider), !pdf.isEncrypted else { return false }
        var visited: Set<UInt> = []
        func resources(_ dictionary: CGPDFDictionaryRef, depth: Int) -> Bool {
            guard depth < 32, visited.insert(UInt(bitPattern: dictionary.rawValue)).inserted else { return false }
            var fonts: CGPDFDictionaryRef?
            if CGPDFDictionaryGetDictionary(dictionary, "Font", &fonts), let fonts {
                let values = FontDictionaryValues()
                CGPDFDictionaryApplyFunction(fonts, collectFontValues, Unmanaged.passUnretained(values).toOpaque())
                for object in values.values {
                    var font: CGPDFDictionaryRef?
                    guard CGPDFObjectGetValue(object, .dictionary, &font), let font else { continue }
                    var descendants: CGPDFArrayRef?
                    guard CGPDFDictionaryGetArray(font, "DescendantFonts", &descendants), let descendants else { continue }
                    for i in 0..<CGPDFArrayGetCount(descendants) {
                        var child: CGPDFDictionaryRef?, descriptor: CGPDFDictionaryRef?, stream: CGPDFStreamRef?
                        var subtype: UnsafePointer<CChar>?
                        var format = CGPDFDataFormat.raw
                        guard CGPDFArrayGetDictionary(descendants, i, &child), let child,
                              CGPDFDictionaryGetName(child, "Subtype", &subtype), let subtype, String(cString: subtype) == "CIDFontType2",
                              CGPDFDictionaryGetDictionary(child, "FontDescriptor", &descriptor), let descriptor,
                              CGPDFDictionaryGetStream(descriptor, "FontFile2", &stream), let stream,
                              let bytes = CGPDFStreamCopyData(stream, &format) as Data?, bytes.count >= 4 else { continue }
                        if bytes[0] == 1, bytes[1] == 0 { return true }
                    }
                }
            }
            var forms: CGPDFDictionaryRef?
            if CGPDFDictionaryGetDictionary(dictionary, "XObject", &forms), let forms {
                let values = FontDictionaryValues()
                CGPDFDictionaryApplyFunction(forms, collectFontValues, Unmanaged.passUnretained(values).toOpaque())
                for object in values.values {
                    var stream: CGPDFStreamRef?, nested: CGPDFDictionaryRef?
                    if CGPDFObjectGetValue(object, .stream, &stream), let stream, let streamDictionary = CGPDFStreamGetDictionary(stream),
                       CGPDFDictionaryGetDictionary(streamDictionary, "Resources", &nested), let nested,
                       resources(nested, depth: depth + 1) { return true }
                }
            }
            return false
        }
        for i in 1...max(1,pdf.numberOfPages) {
            var page = pdf.page(at: i)?.dictionary
            for _ in 0..<32 {
                guard let current = page else { break }
                var dictionary: CGPDFDictionaryRef?
                if CGPDFDictionaryGetDictionary(current, "Resources", &dictionary), let dictionary,
                   resources(dictionary, depth: 0) { return true }
                var parent: CGPDFDictionaryRef?
                _ = CGPDFDictionaryGetDictionary(current, "Parent", &parent); page = parent
            }
        }
        return false
    }
}
