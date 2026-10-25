import AppKit
import Foundation
import PDFKit

struct NativeContentItem {
    enum Kind { case text, image, path }
    let kind: Kind
    let stream: String
    let range: Range<Int>
    let reference: String?
    let text: String
    let origin: CGPoint
    let font: String
    let size: Double
    let color: Int
    var bounds: CGRect
    var editable: Bool
    // Semantic identity survives PDFKit object renumbering when the workspace
    // creates its immutable engine snapshot. Ambiguous duplicates are rejected.
    var id: String {
        let type = kind == .text ? "text" : kind == .image ? "image" : "path"
        let numbers = [Double(origin.x),Double(origin.y),size,Double(bounds.minX),Double(bounds.minY),Double(bounds.width),Double(bounds.height)].map { String(format:"%.4f",$0) }
        return NativeContentGraph.identity([type,text,font,String(color)] + numbers)
    }
    var location: String { "\(stream):\(range.lowerBound):\(range.upperBound)" }
}

extension NativeContentGraph {
    func scan(page index: Int, contents overrideContents: [String]? = nil, resources overrideResources: [String:Any]? = nil, transform: CGAffineTransform = .identity, clip inheritedClip: CGRect? = nil, textClipped: Bool = false, ancestors: Set<String> = []) throws -> [NativeContentItem] {
        guard (0..<pages.count).contains(index), let page = document.page(at: index), let resources = overrideResources ?? resources(at: index), ancestors.count <= 16 else { throw OrganizeError.invalidInput }
        let fonts = dictionary(resources["/Font"]) ?? [:], xobjects = dictionary(resources["/XObject"]) ?? [:]
        var output: [NativeContentItem] = []
        var ctm = transform
        var stack: [(CGAffineTransform,Int,String,Double,Int,Bool,Bool,CGRect,Bool,Double,Double)] = []
        var hasClip = textClipped, clip = inheritedClip ?? page.bounds(for:.mediaBox)
        var simpleText = true, lineWidth = 1.0, miterLimit = 10.0
        var textMatrix = CGAffineTransform.identity, lineMatrix = CGAffineTransform.identity
        var fontName = "", fontSize = 12.0, fill = 0, leading = 0.0
        var insideText = false, textStart = 0, textItems: [Int] = [], mode = 0, opacityKnown = true
        let contents = overrideContents ?? (pages[index]["contents"] as? [String] ?? [])
        var seen = Set<String>()
        for reference in contents {
            guard seen.insert(reference).inserted else { throw OrganizeError.processFailed("共享内容流不支持原文编辑") }
            let data = try stream(reference)
            var lexer = NativePDFLexer(data), operands: [NativePDFToken] = []
            var path = CGRect.null, pathStart: Int?
            while let token = try lexer.next() {
                if cancelled() { throw OrganizeError.cancelled }
                if let name = token.name, ["/ActualText","/Alt","/OC","/OCG","/OCMD"].contains(name) { throw OrganizeError.processFailed("替代文字或可选层内容不能安全原文编辑；未修改文档") }
                guard case .word(let operation) = token.value else { operands.append(token); continue }
                defer { operands.removeAll(keepingCapacity: true) }
                let numbers = operands.compactMap(\.number)
                switch operation {
                case "q": guard stack.count < 64 else { throw OrganizeError.invalidInput }; stack.append((ctm,fill,fontName,fontSize,mode,opacityKnown,hasClip,clip,simpleText,lineWidth,miterLimit))
                case "Q": guard let previous = stack.popLast() else { throw OrganizeError.invalidInput }; (ctm,fill,fontName,fontSize,mode,opacityKnown,hasClip,clip,simpleText,lineWidth,miterLimit) = previous
                case "cm": guard numbers.count == 6 else { throw OrganizeError.invalidInput }; ctm = CGAffineTransform(a:numbers[0],b:numbers[1],c:numbers[2],d:numbers[3],tx:numbers[4],ty:numbers[5]).concatenating(ctm)
                case "BT": guard !insideText else { throw OrganizeError.invalidInput }; insideText = true; textStart = token.start; textItems = []; textMatrix = .identity; lineMatrix = .identity
                case "ET":
                    guard insideText else { throw OrganizeError.invalidInput }
                    if textItems.count != 1 { for item in textItems { output[item].editable = false } }
                    insideText = false
                case "Tf": guard operands.count == 2, let name = operands[0].name, let size = operands[1].number, size > 0, size <= 4096 else { throw OrganizeError.invalidInput }; fontName = name; fontSize = size
                case "Tm": guard numbers.count == 6 else { throw OrganizeError.invalidInput }; textMatrix = CGAffineTransform(a:numbers[0],b:numbers[1],c:numbers[2],d:numbers[3],tx:numbers[4],ty:numbers[5]); lineMatrix = textMatrix
                case "Td", "TD":
                    guard numbers.count == 2 else { throw OrganizeError.invalidInput }
                    if operation == "TD" { leading = -numbers[1] }
                    lineMatrix = CGAffineTransform(translationX: numbers[0], y: numbers[1]).concatenating(lineMatrix); textMatrix = lineMatrix
                case "TL": if let number = numbers.first { leading = number }
                case "T*": lineMatrix = CGAffineTransform(translationX: 0, y: -leading).concatenating(lineMatrix); textMatrix = lineMatrix
                case "Tr": mode = Int(numbers.first ?? -1)
                case "Tc", "Tw", "Ts": if numbers.count != 1 || numbers[0] != 0 { simpleText = false }
                case "Tz": if numbers.count != 1 || numbers[0] != 100 { simpleText = false }
                case "w": if let value = numbers.first { lineWidth = abs(value) }
                case "M": if let value = numbers.first { miterLimit = max(1,value) }
                case "rg": if numbers.count == 3 { fill = Self.rgb(numbers) }
                case "g": if let gray = numbers.first { fill = Self.rgb([gray, gray, gray]) }
                case "k": if numbers.count == 4 { fill = Self.rgb([1-min(1,numbers[0]+numbers[3]),1-min(1,numbers[1]+numbers[3]),1-min(1,numbers[2]+numbers[3])]) }
                case "sc", "scn":
                    if numbers.count == 3 { fill = Self.rgb(numbers) }
                    else if numbers.count == 1 { fill = Self.rgb([numbers[0],numbers[0],numbers[0]]) }
                    else { opacityKnown = false }
                case "gs":
                    if let name = operands.first?.name, let ext = dictionary(resources["/ExtGState"]), let state = dictionary(ext[name]) {
                        opacityKnown = ((state["/ca"] as? NSNumber)?.doubleValue ?? 1) == 1 && state["/SMask"] == nil
                    } else { opacityKnown = false }
                case "Tj", "TJ", "'", "\"":
                    guard insideText, let bytes = operands.last?.textBytes, let font = dictionary(fonts[fontName]) else { throw OrganizeError.processFailed("不支持的原文编码") }
                    if operation == "'" || operation == "\"" { lineMatrix = CGAffineTransform(translationX: 0, y: -leading).concatenating(lineMatrix); textMatrix = lineMatrix }
                    if operation == "\"", numbers.contains(where: { $0 != 0 }) { simpleText = false }
                    let string = try decode(bytes, font: font)
                    let matrix = textMatrix.concatenating(ctm), origin = CGPoint(x:matrix.tx,y:matrix.ty)
                    let size = fontSize * hypot(matrix.a, matrix.b)
                    let realFont = (font["/BaseFont"] as? String ?? "/Helvetica").dropFirst().description
                    let horizontal = abs(matrix.b) < 0.00001 && abs(matrix.c) < 0.00001 && matrix.a > 0 && matrix.d > 0
                    let measured = textBounds(string, origin: origin, fontName: realFont, size: size, on: page)
                    output.append(NativeContentItem(kind: .text, stream: reference, range: (operands.first?.start ?? token.start)..<token.end,
                        reference: nil, text: string, origin: origin, font: realFont.components(separatedBy:"+").last ?? realFont, size: size, color: fill,
                        bounds: measured.0.intersection(clip), editable: horizontal && mode == 0 && opacityKnown && simpleText && !hasClip && !string.isEmpty))
                    textItems.append(output.count - 1)
                    // A later showing operator in this BT changes placement. We can
                    // inspect it, but fail-closed editing requires one independent show.
                    textMatrix.tx += measured.0.width
                case "Do":
                    guard let name = operands.first?.name, let objectRef = xobjects[name] as? String, let object = dictionary(objectRef) else { throw OrganizeError.invalidInput }
                    if object["/Subtype"] as? String == "/Form" {
                        guard !ancestors.contains(objectRef), object["/Group"] == nil, let rect = object["/BBox"] as? [NSNumber], rect.count == 4 else { throw OrganizeError.processFailed("循环或透明组 Form 暂不支持安全编辑") }
                        let numbers = object["/Matrix"] as? [NSNumber] ?? [1,0,0,1,0,0]
                        guard numbers.count == 6 else { throw OrganizeError.invalidInput }
                        let matrix = CGAffineTransform(a:numbers[0].doubleValue,b:numbers[1].doubleValue,c:numbers[2].doubleValue,d:numbers[3].doubleValue,tx:numbers[4].doubleValue,ty:numbers[5].doubleValue).concatenating(ctm)
                        let box = CGRect(x:rect[0].doubleValue,y:rect[1].doubleValue,width:rect[2].doubleValue-rect[0].doubleValue,height:rect[3].doubleValue-rect[1].doubleValue).applying(matrix).intersection(clip)
                        var seen = ancestors; seen.insert(objectRef)
                        output += try scan(page:index,contents:[objectRef],resources:dictionary(object["/Resources"]) ?? resources,transform:matrix,clip:box,textClipped:hasClip,ancestors:seen)
                        continue
                    }
                    guard object["/Subtype"] as? String == "/Image" else { throw OrganizeError.processFailed("未知 XObject 暂不支持安全编辑") }
                    let bounds = CGRect(x:0,y:0,width:1,height:1).applying(ctm).intersection(clip)
                    output.append(NativeContentItem(kind: .image, stream: reference, range: (operands.first?.start ?? token.start)..<token.end,
                        reference: objectRef, text: "", origin: .zero, font: "", size: 0, color: 0, bounds: bounds,
                        editable: opacityKnown && object["/SMask"] == nil && object["/Mask"] == nil && object["/ImageMask"] == nil))
                case "m", "l":
                    if pathStart == nil { pathStart = operands.first?.start ?? token.start }
                    if numbers.count == 2 { let p = CGPoint(x:numbers[0],y:numbers[1]).applying(ctm); path = path.union(CGRect(x:p.x,y:p.y,width:0.001,height:0.001)) }
                case "c", "v", "y":
                    if pathStart == nil { pathStart = operands.first?.start ?? token.start }
                    for n in stride(from: 0, to: numbers.count - numbers.count % 2, by: 2) { let p = CGPoint(x:numbers[n],y:numbers[n+1]).applying(ctm); path = path.union(CGRect(x:p.x,y:p.y,width:0.001,height:0.001)) }
                case "re": if numbers.count == 4 { if pathStart == nil { pathStart = operands.first?.start ?? token.start }; path = path.union(CGRect(x:numbers[0],y:numbers[1],width:numbers[2],height:numbers[3]).applying(ctm)) }
                case "S", "s", "f", "F", "f*", "B", "B*", "b", "b*":
                    if let pathStart, !path.isNull {
                        let strokes = ["S","s","B","B*","b","b*"].contains(operation)
                        let radius = strokes ? max(0.5,lineWidth*max(hypot(ctm.a,ctm.b),hypot(ctm.c,ctm.d))*miterLimit/2) : 0
                        output.append(NativeContentItem(kind:.path,stream:reference,range:pathStart..<token.end,reference:nil,text:"",origin:.zero,font:"",size:0,color:0,bounds:path.insetBy(dx:-radius,dy:-radius).intersection(clip),editable:!hasClip))
                    }
                    path = .null; pathStart = nil
                case "n": path = .null; pathStart = nil
                case "W", "W*": hasClip = true; clip = clip.intersection(path)
                case "BI", "ID", "EI", "sh": throw OrganizeError.processFailed("内联图像或着色内容暂不支持安全原文编辑")
                default: break
                }
            }
            guard !insideText else { throw OrganizeError.invalidInput }
            _ = textStart
        }
        guard stack.isEmpty, output.count <= 30000 else { throw OrganizeError.invalidInput }
        let counts = Dictionary(grouping:pages.flatMap { $0["contents"] as? [String] ?? [] },by:{ $0 }).mapValues(\.count)
        for index in output.indices where (counts[output[index].stream] ?? 0) > 1 { output[index].editable = false }
        return output
    }
    private static func rgb(_ values: [Double]) -> Int { values.reduce(0) { ($0 << 8) | Int((max(0,min(1,$1))*255).rounded()) } }
    private func textBounds(_ text: String, origin: CGPoint, fontName: String, size: Double, on page: PDFPage) -> (CGRect, String) {
        let fallback = NSFont(name: fontName.components(separatedBy: "+").last ?? fontName, size: size) ?? .systemFont(ofSize:size)
        let full = page.string ?? "", ns = full as NSString
        var search = NSRange(location:0,length:ns.length), best: (CGRect,String,Double)?
        while search.length > 0 {
            let range = ns.range(of:text,options:[],range:search)
            guard range.location != NSNotFound else { break }
            if let selection = page.selection(for:range), !selection.bounds(for:page).isEmpty {
                let bounds = selection.bounds(for:page)
                let font = (page.attributedString?.attribute(.font, at:range.location, effectiveRange:nil) as? NSFont)?.fontName ?? fallback.fontName
                let distance = abs(bounds.minX-origin.x) + abs(bounds.minY-fallback.descender-origin.y)
                if best == nil || distance < best!.2 { best = (bounds,font,distance) }
            }
            search = NSRange(location:range.location+max(1,range.length),length:max(0,ns.length-range.location-max(1,range.length)))
        }
        if let best { return (best.0,best.1) }
        let width = (text as NSString).size(withAttributes:[.font:fallback]).width
        return (CGRect(x:origin.x,y:origin.y+fallback.descender,width:max(0.01,width),height:fallback.ascender-fallback.descender),fallback.fontName)
    }
    private func decode(_ data: Data, font: [String: Any]) throws -> String {
        guard data.count <= 100000 else { throw OrganizeError.invalidInput }
        if let cmapRef = font["/ToUnicode"] as? String {
            let cmap = String(decoding: try stream(cmapRef), as:UTF8.self)
            var mapping: [Data:String] = [:], widths = Set<Int>()
            func hex(_ text: Substring) -> Data? { var value = Data(); let chars = Array(text); guard chars.count % 2 == 0 else { return nil }; for i in stride(from:0,to:chars.count,by:2) { guard let b = UInt8(String(chars[i...i+1]),radix:16) else { return nil }; value.append(b) }; return value }
            func matches(_ pattern: String, in source: String) -> [[Substring]] {
                guard let regex = try? NSRegularExpression(pattern:pattern,options:[.dotMatchesLineSeparators]) else { return [] }
                return regex.matches(in:source,range:NSRange(source.startIndex...,in:source)).map { match in
                    (0..<match.numberOfRanges).map { index in Range(match.range(at:index),in:source).map { source[$0] } ?? "" }
                }
            }
            for section in matches("beginbfchar(.*?)endbfchar",in:cmap) {
                for pair in matches("<([0-9A-Fa-f]+)>\\s*<([0-9A-Fa-f]+)>",in:String(section[1])) {
                    if let key = hex(pair[1]), let value = hex(pair[2]), let string = String(data:value,encoding:.utf16BigEndian) { mapping[key] = string; widths.insert(key.count) }
                }
            }
            for section in matches("beginbfrange(.*?)endbfrange",in:cmap) {
                for row in matches("<([0-9A-Fa-f]+)>\\s*<([0-9A-Fa-f]+)>\\s*<([0-9A-Fa-f]+)>",in:String(section[1])) {
                    guard let start = UInt32(row[1],radix:16), let end = UInt32(row[2],radix:16), let target = UInt32(row[3],radix:16), end >= start, end-start <= 65535, row[1].count <= 8, row[3].count == 4 else { continue }
                    let count = row[1].count/2; widths.insert(count)
                    for code in start...end {
                        var key = Data(); for i in (0..<count).reversed() { key.append(UInt8((code >> (i*8))&255)) }
                        let scalar = target+(code-start)
                        if let u = UnicodeScalar(scalar) { mapping[key] = String(u) }
                    }
                }
            }
            var result = "", offset = 0; let widthsDescending = widths.sorted(by:>)
            while offset < data.count {
                var found = false
                for width in widthsDescending where offset+width <= data.count {
                    if let value = mapping[data.subdata(in:offset..<offset+width)] { result += value; offset += width; found = true; break }
                }
                guard found else { throw OrganizeError.processFailed("原文字符映射不完整；未修改文档") }
            }
            return result
        }
        if font["/Subtype"] as? String == "/Type0" {
            guard font["/Encoding"] as? String == "/UniGB-UTF16-H", let result = String(data:data,encoding:.utf16BigEndian) else { throw OrganizeError.processFailed("不支持的 CID 字符映射") }
            return result
        }
        if dictionary(font["/Encoding"])?["/Differences"] != nil { throw OrganizeError.processFailed("自定义简单字体编码暂不支持原文编辑") }
        guard let result = String(data:data,encoding:.windowsCP1252) ?? String(data:data,encoding:.isoLatin1) else { throw OrganizeError.invalidInput }
        return result
    }
}
