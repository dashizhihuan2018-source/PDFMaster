import Foundation
import Darwin
import AppKit
import PDFKit
import ImageIO

public struct ContentSpan: Codable { public let id: String; public let text: String; public let bbox: [Double]; public let origin: [Double]; public let font: String; public let size: Double; public let color: Int }
public struct ContentImage: Codable { public let id: String; public let xref: Int; public let bbox: [Double] }
public struct ContentLink: Codable { public let id: String; public let bbox: [Double]; public let uri: String }
public struct ContentPage: Codable {
    public let page: Int; public let width: Double; public let height: Double
    public let spans: [ContentSpan]; public let images: [ContentImage]; public let links: [ContentLink]; public let limitations: [String]
}
/// An explicit uniform style for one existing horizontal span. This does not
/// enable paragraph reflow: the content engine still checks its actual glyph
/// bounds against the original footprint, page and all neighboring objects.
public struct ContentTextStyle: Codable, Equatable {
    public let fontName: String
    public let size: Double
    public let color: Int
    /// Only the unchanged source face may use the embedded-font fallback. Nil
    /// keeps older encoded requests and explicit font selections strict.
    public let retainsOriginalFont: Bool?
    public init(fontName: String, size: Double, color: Int, retainsOriginalFont: Bool = false) {
        self.fontName = fontName; self.size = size; self.color = color
        self.retainsOriginalFont = retainsOriginalFont ? true : nil
    }
    var isValid: Bool {
        !fontName.isEmpty && fontName.utf8.count <= 256 &&
        !fontName.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) &&
        size.isFinite && (1...512).contains(size) && (0...0xFFFFFF).contains(color)
    }
}
public struct ContentEditRequest: Encodable {
    public let operation: String; public let page: Int
    public var id: String?; public var text: String?; public var bbox: [Double]?; public var uri: String?; public var image_path: String?
    public var textStyle: ContentTextStyle?
    var rendered_text_pdf_path: String?
    public init(operation: String, page: Int) { self.operation = operation; self.page = page }
}

/// Native content editing. Apple frameworks supply text layout and image decoding;
/// bundled Apache-2.0 qpdf supplies lossless PDF object transport. No Python runtime.
public final class ContentEditEngine {
    // Compatibility arguments are ignored; neither a runtime path nor a script
    // is retained. Existing integrations can migrate without loading old code.
    public init(python: URL? = nil, script: URL? = nil) {}

    private var qpdf: URL {
        var candidates: [URL] = []
        if let resources = Bundle.main.resourceURL { candidates.append(resources.appendingPathComponent("Engines/qpdf/bin/qpdf")) }
        #if DEBUG
        if let path = ProcessInfo.processInfo.environment["PDFMASTER_TEST_QPDF"], path.hasPrefix("/") { candidates.append(URL(fileURLWithPath: path)) }
        let sourceRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        candidates.append(sourceRoot.appendingPathComponent("tools/qpdf-arm64-native-jpeg8-12.4.2/bin/qpdf"))
        #endif
        return candidates.first(where: { FileManager.default.isExecutableFile(atPath: $0.path) }) ?? URL(fileURLWithPath: "/unavailable/qpdf")
    }

    public func validateAdmission(_ data: Data, in directory: URL, cancelled: () -> Bool) throws {
        let source = directory.appendingPathComponent("admission-" + UUID().uuidString + ".pdf")
        guard FileManager.default.createFile(atPath: source.path, contents: data, attributes: [.posixPermissions: 0o600]) else { throw OrganizeError.invalidInput }
        defer { try? FileManager.default.removeItem(at: source) }
        _ = try inspect(source, page: 0, cancelled: cancelled)
    }

    public func inspect(_ source: URL, page: Int, cancelled: () -> Bool) throws -> ContentPage {
        try withoutActuallyEscaping(cancelled) { check in
            let graph = try NativeContentGraph(input: source, qpdf: qpdf, cancelled: check)
            return try inspect(graph, page: page)
        }
    }
    public func apply(_ source: URL, request: ContentEditRequest, output: URL, cancelled: () -> Bool) throws -> String {
        try withoutActuallyEscaping(cancelled) { check in
            guard !FileManager.default.fileExists(atPath: output.path), request.operation != "inspect",
                  request.textStyle == nil || (request.operation == "replace-text" && request.textStyle!.isValid) else { throw OrganizeError.invalidInput }
            let graph = try NativeContentGraph(input: source, qpdf: qpdf, cancelled: check)
            guard (0..<graph.pages.count).contains(request.page) else { throw OrganizeError.invalidInput }
            if ["insert-link", "update-link", "delete-link"].contains(request.operation) {
                try editLinks(graph, request: request)
                try graph.rewrite(output: output)
                return "链接已更新 · 可撤销 · 请存储副本"
            }
            let items = try graph.scan(page: request.page)
            let page = try inspect(graph, page: request.page, items: items)
            var removals: [NativeContentItem] = [], overlay: URL?, font: String?
            var deletedText = "", replacement = ""
            switch request.operation {
            case "replace-text":
                guard items.filter({ $0.kind == .text && $0.id == request.id }).count == 1,
                      let item = items.first(where: { $0.kind == .text && $0.id == request.id }), item.editable,
                      let text = request.text, text.utf8.count <= 16384,
                      let span = page.spans.first(where: { $0.id == item.id }) else { throw OrganizeError.processFailed("此文字对象排版复杂，不能安全替换；未修改文档") }
                deletedText = item.text; replacement = text; removals = [item]
                if !text.isEmpty {
                    let rendered = try NativeContentTextPDF.render(text, span: span, page: page, style: request.textStyle)
                    guard let renderedDoc = PDFDocument(data: rendered.0), let renderedPage = renderedDoc.page(at: 0),
                          let range = (renderedPage.string as NSString?)?.range(of: text), range.location != NSNotFound,
                          let selection = renderedPage.selection(for: range) else { throw OrganizeError.processFailed("原生排版结果不可检索；未修改文档") }
                    // PDFKit's selection rectangles are font ascent/descent boxes,
                    // not visible glyph footprints (CJK fallback can be 1.4 em).
                    // Check actual native glyph paths while retaining the baseline.
                    let bounds = rendered.2, allowed = item.bounds.insetBy(dx: -1, dy: -2)
                    guard !selection.bounds(for:renderedPage).isEmpty else { throw OrganizeError.invalidInput }
                    guard allowed.contains(bounds), CGRect(x:0,y:0,width:page.width,height:page.height).contains(bounds),
                          !items.contains(where: { $0.id != item.id && $0.kind != .path && $0.bounds.intersects(bounds.insetBy(dx:0.2,dy:0.2)) }) else { throw OrganizeError.processFailed("新文字超出原文字范围或碰到邻近对象；未修改文档（原范围 \(item.bounds)，新范围 \(bounds)）") }
                    let url = graph.job.appendingPathComponent("native-text.pdf")
                    try rendered.0.write(to: url, options: [.withoutOverwriting]); overlay = url; font = rendered.1
                }
            case "replace-image":
                guard let item = items.first(where: { $0.kind == .image && $0.id == request.id }), item.editable,
                      let ref = item.reference, let path = request.image_path else { throw OrganizeError.invalidInput }
                var uses = 0
                for index in graph.pages.indices { uses += try graph.scan(page:index).filter { $0.reference == ref }.count }
                guard uses == 1 else { throw OrganizeError.processFailed("共享图片不能安全单独替换；未修改文档") }
                try replaceImage(graph, reference: ref, url: URL(fileURLWithPath: path))
            case "redact":
                let rect = try nativeRect(request.bbox, page: page)
                removals = items.filter { $0.bounds.intersects(rect) }
                guard !removals.isEmpty, removals.allSatisfy(\.editable) else { throw OrganizeError.processFailed("选区包含不支持的对象；未删除任何内容") }
                // Non-link annotations may have appearance streams with content.
                guard graph.document.page(at:request.page)!.annotations.allSatisfy({ $0.type == "Link" || !$0.bounds.intersects(rect) }) else { throw OrganizeError.processFailed("选区包含批注外观，不能安全移除") }
                try removeLinks(graph, page:request.page, intersecting:rect)
            default: throw OrganizeError.invalidInput
            }
            if !removals.isEmpty {
                var locations: [String:Int] = [:], imageUses: [String:Int] = [:]
                for index in graph.pages.indices {
                    for item in try graph.scan(page:index) {
                        locations[item.location,default:0] += 1
                        if let ref = item.reference { imageUses[ref,default:0] += 1 }
                    }
                }
                func references(_ value:Any, to ref:String) -> Int {
                    if let text = value as? String { return text == ref ? 1 : 0 }
                    if let dict = value as? [String:Any] { return dict.values.reduce(0) { $0+references($1,to:ref) } }
                    if let array = value as? [Any] { return array.reduce(0) { $0+references($1,to:ref) } }
                    return 0
                }
                guard removals.allSatisfy({ locations[$0.location] == 1 }) else { throw OrganizeError.processFailed("共享内容对象不能安全单独删除；未修改文档") }
                for image in removals where image.kind == .image {
                    guard let ref = image.reference, imageUses[ref] == 1, references(graph.objects,to:ref) == 1 else { throw OrganizeError.processFailed("共享图片资源不能安全删除；未修改文档") }
                    // Remove pixel bytes as well as the paint operator. A valid
                    // empty resource prevents stale resource dictionaries from
                    // retaining an extractable image in the saved PDF.
                    graph.setStream(ref,bytes:Data(),dictionary:["/Type":"/XObject","/Subtype":"/Form","/BBox":[0,0,1,1],"/Resources":[:]])
                }
            }
            for group in Dictionary(grouping: removals, by: \.stream) {
                var data = try graph.stream(group.key)
                for item in group.value.sorted(by: { $0.range.lowerBound > $1.range.lowerBound }) { data.replaceSubrange(item.range, with: Data()) }
                graph.setStream(group.key, bytes: data)
            }
            try graph.rewrite(output: output, overlay: overlay, overlayPage: request.page) { candidate in
                for index in graph.pages.indices where index != request.page {
                    guard candidate.page(at:index)?.string == graph.document.page(at:index)?.string else { throw OrganizeError.processFailed("非目标页文字一致性校验失败") }
                }
                if request.operation == "replace-text" {
                    let before = graph.document.page(at:request.page)?.string ?? "", after = candidate.page(at:request.page)?.string ?? ""
                    func occurrences(_ text: String, in value: String) -> Int { text.isEmpty ? 0 : value.components(separatedBy:text).count-1 }
                    guard occurrences(deletedText,in:after) == occurrences(deletedText,in:before)-1 + (replacement == deletedText ? 1 : 0),
                          replacement.isEmpty || after.contains(replacement) else { throw OrganizeError.processFailed("原文真实移除或新文字可检索校验失败") }
                }
            }
            if let font { return "原生排版文字已写入 PDF · 实际字体：" + font }
            return "内容编辑已完成 · 可撤销 · 请存储副本"
        }
    }
    private func inspect(_ graph: NativeContentGraph, page index: Int, items supplied: [NativeContentItem]? = nil) throws -> ContentPage {
        guard (0..<graph.pages.count).contains(index), let pdfPage = graph.document.page(at:index) else { throw OrganizeError.invalidInput }
        let bounds = pdfPage.bounds(for:.mediaBox), items = try supplied ?? graph.scan(page:index)
        func box(_ rect: CGRect) -> [Double] { [rect.minX,bounds.height-rect.maxY,rect.maxX,bounds.height-rect.minY].map(Double.init) }
        let spans = items.filter { $0.kind == .text && !$0.text.isEmpty }.map { ContentSpan(id:$0.id,text:$0.text,bbox:box($0.bounds),origin:[Double($0.origin.x),Double(bounds.height-$0.origin.y)],font:$0.font,size:$0.size,color:$0.color) }
        let images = items.filter { $0.kind == .image }.map { ContentImage(id:$0.id,xref:Int(($0.reference ?? "0").split(separator:" ").first ?? "0") ?? 0,bbox:box($0.bounds)) }
        let links = try linkObjects(graph,page:index).map { ContentLink(id:$0.0,bbox:box($0.2),uri:$0.3) }
        return ContentPage(page:index,width:Double(bounds.width),height:Double(bounds.height),spans:spans,images:images,links:links,limitations:["独立原生引擎：复杂复合或裁剪文字、标签替代文字、内联图像、透明组 Form、共享对象会保守拒绝；不提供段落重排"])
    }
    private func nativeRect(_ array: [Double]?, page: ContentPage) throws -> CGRect {
        guard let a = array, a.count == 4, a.allSatisfy(\.isFinite), a[0] >= 0, a[1] >= 0, a[2] > a[0], a[3] > a[1], a[2] <= page.width, a[3] <= page.height else { throw OrganizeError.invalidInput }
        return CGRect(x:a[0],y:page.height-a[3],width:a[2]-a[0],height:a[3]-a[1])
    }
    private func linkObjects(_ graph: NativeContentGraph, page: Int) throws -> [(String,[String:Any],CGRect,String)] {
        let (_, dictionary) = try graph.pageDictionary(at:page)
        let array = try annotations(graph,in:dictionary)
        return try array.compactMap { item in
            guard let ref = item as? String, let value = graph.dictionary(ref) else { throw OrganizeError.processFailed("内联批注暂不支持安全链接编辑") }
            guard value["/Subtype"] as? String == "/Link" else { return nil }
            guard let a = value["/Rect"] as? [NSNumber], a.count == 4 else { throw OrganizeError.invalidInput }
            let rect = CGRect(x:a[0].doubleValue,y:a[1].doubleValue,width:a[2].doubleValue-a[0].doubleValue,height:a[3].doubleValue-a[1].doubleValue)
            let action = graph.dictionary(value["/A"]), encoded = action?["/URI"] as? String ?? ""
            return (ref,value,rect,encoded.hasPrefix("u:") ? String(encoded.dropFirst(2)) : "")
        }
    }
    private func annotations(_ graph: NativeContentGraph, in page: [String:Any]) throws -> [Any] {
        guard let value = page["/Annots"] else { return [] }
        if let array = value as? [Any] { return array }
        if let reference = value as? String, let object = graph.objects["obj:"+reference] as? [String:Any], let array = object["value"] as? [Any] { return array }
        throw OrganizeError.processFailed("批注数组无效；未修改原有批注")
    }
    private func validURI(_ value: String?) throws -> String {
        guard let value, value.utf8.count <= 4096, !value.unicodeScalars.contains(where: { CharacterSet.whitespacesAndNewlines.contains($0) || CharacterSet.controlCharacters.contains($0) }),
              let components = URLComponents(string:value), ["https","http","mailto"].contains(components.scheme?.lowercased() ?? ""), components.user == nil, components.password == nil,
              components.scheme?.lowercased() == "mailto" ? !components.path.isEmpty : !(components.host ?? "").isEmpty else { throw OrganizeError.invalidInput }
        return value
    }
    private func editLinks(_ graph: NativeContentGraph, request: ContentEditRequest) throws {
        let (ref, original) = try graph.pageDictionary(at:request.page); var page = original
        var annots = try annotations(graph,in:page)
        if request.operation == "insert-link" {
            let bounds = graph.document.page(at:request.page)!.bounds(for:.mediaBox)
            let model = ContentPage(page:request.page,width:Double(bounds.width),height:Double(bounds.height),spans:[],images:[],links:[],limitations:[])
            let rect = try nativeRect(request.bbox,page:model), uri = try validURI(request.uri)
            let next = (graph.header["maxobjectid"] as? Int ?? graph.objects.count)+1, newRef = "\(next) 0 R"
            graph.setObject(newRef,value:["/Type":"/Annot","/Subtype":"/Link","/Rect":[rect.minX,rect.minY,rect.maxX,rect.maxY],"/Border":[0,0,0],"/A":["/S":"/URI","/URI":"u:"+uri]])
            annots.append(newRef)
        } else {
            guard let link = try linkObjects(graph,page:request.page).first(where: { $0.0 == request.id }) else { throw OrganizeError.invalidInput }
            if request.operation == "delete-link" { annots.removeAll { $0 as? String == link.0 } }
            else { var value = link.1; value["/A"] = ["/S":"/URI","/URI":"u:"+(try validURI(request.uri))]; value.removeValue(forKey:"/Dest"); graph.setObject(link.0,value:value) }
        }
        page["/Annots"] = annots; graph.setObject(ref,value:page)
    }
    private func removeLinks(_ graph: NativeContentGraph, page index: Int, intersecting rect: CGRect) throws {
        let deleted = Set(try linkObjects(graph,page:index).filter { $0.2.intersects(rect) }.map { $0.0 })
        guard !deleted.isEmpty else { return }
        let (ref, original) = try graph.pageDictionary(at:index); var page = original
        page["/Annots"] = try annotations(graph,in:page).filter { !deleted.contains($0 as? String ?? "") }; graph.setObject(ref,value:page)
    }
    private func replaceImage(_ graph: NativeContentGraph, reference: String, url: URL) throws {
        guard let values = try? url.resourceValues(forKeys:[.isRegularFileKey,.isSymbolicLinkKey,.fileSizeKey]), values.isRegularFile == true, values.isSymbolicLink != true,
              (values.fileSize ?? Int.max) <= 128*1024*1024, let source = CGImageSourceCreateWithURL(url as CFURL,nil), CGImageSourceGetCount(source) == 1,
              let info = CGImageSourceCopyPropertiesAtIndex(source,0,nil) as? [CFString:Any], let width = info[kCGImagePropertyPixelWidth] as? Int, let height = info[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, width <= 16384, height <= 16384, width*height <= 40_000_000, let image = CGImageSourceCreateImageAtIndex(source,0,[kCGImageSourceShouldCacheImmediately:true] as CFDictionary),
              [.none,.noneSkipFirst,.noneSkipLast].contains(image.alphaInfo) else { throw OrganizeError.processFailed("仅支持预算内的不透明单帧 PNG/JPEG；未修改文档") }
        var rgba = Data(count:width*height*4)
        let success = rgba.withUnsafeMutableBytes { pointer -> Bool in
            guard let context = CGContext(data:pointer.baseAddress,width:width,height:height,bitsPerComponent:8,bytesPerRow:width*4,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.noneSkipLast.rawValue) else { return false }
            context.draw(image,in:CGRect(x:0,y:0,width:width,height:height)); return true
        }
        guard success else { throw OrganizeError.invalidInput }
        var rgb = Data(capacity:width*height*3)
        rgba.withUnsafeBytes { pointer in let bytes = pointer.bindMemory(to:UInt8.self); for index in stride(from:0,to:bytes.count,by:4) { rgb.append(contentsOf:bytes[index..<index+3]) } }
        graph.setStream(reference,bytes:rgb,dictionary:["/Type":"/XObject","/Subtype":"/Image","/Width":width,"/Height":height,"/ColorSpace":"/DeviceRGB","/BitsPerComponent":8])
    }
}
