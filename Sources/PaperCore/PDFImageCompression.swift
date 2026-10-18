import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

/// Changes only admitted Image XObject streams, never page content or annotation objects.
/// Unknown graphics/resource constructions conservatively disable image rewriting.
enum PDFImageCompression {
    static let maximumImagePixels = 32_000_000
    static let maximumTotalPixels = 100_000_000
    static let maximumStreamBytes = 128 * 1024 * 1024

    static func prepare(input: URL, inspection: [String: Any], job: URL, dpi: Int, quality: Int,
                        cancelled: () -> Bool,
                        run: ([String], URL?, Int) throws -> Data) throws -> URL? {
        try withoutActuallyEscaping(cancelled) { cancellation in
            try prepareSynchronously(input: input, inspection: inspection, job: job, dpi: dpi,
                                     quality: quality, cancelled: cancellation, run: run)
        }
    }

    private static func prepareSynchronously(input: URL, inspection: [String: Any], job: URL, dpi: Int, quality: Int,
                        cancelled: @escaping () -> Bool,
                        run: ([String], URL?, Int) throws -> Data) throws -> URL? {
        guard let entries = inspection["qpdf"] as? [Any], entries.count == 2,
              let header = entries[0] as? [String: Any], let objects = entries[1] as? [String: Any],
              let pages = inspection["pages"] as? [[String: Any]],
              let document = CGPDFDocument(input as CFURL), document.numberOfPages == pages.count else { return nil }
        let usage = ImageUsage(objects: objects, cancelled: cancelled)
        usage.excludeUnsupportedImages()
        guard !usage.failed else { return nil }
        for (index, pageInfo) in pages.enumerated() {
            if cancelled() { throw OrganizeError.cancelled }
            guard let reference = pageInfo["object"] as? String,
                  let pageDictionary = usage.dictionary(reference),
                  let resources = usage.pageResources(pageDictionary),
                  ((pageDictionary["/UserUnit"] as? NSNumber)?.doubleValue ?? 1) == 1,
                  let page = document.page(at: index + 1) else { return nil }
            let stream = CGPDFContentStreamCreateWithPage(page)
            defer { CGPDFContentStreamRelease(stream) }
            usage.scan(stream: stream, resources: resources, matrix: .identity, path: [])
            if usage.failed { break }
        }
        if cancelled() { throw OrganizeError.cancelled }
        guard !usage.failed else { return nil }
        var replacements: [String: Any] = [:], totalPixels = 0
        for reference in usage.sizes.keys.sorted() {
            if cancelled() { throw OrganizeError.cancelled }
            guard !usage.excludedImages.contains(reference),
                  let required = usage.sizes[reference], let dictionary = usage.dictionary(reference),
                  dictionary["/Subtype"] as? String == "/Image",
                  let w = dictionary["/Width"] as? Int, let h = dictionary["/Height"] as? Int,
                  w > 0, h > 0, w <= 20_000, h <= 20_000, w <= maximumImagePixels / h,
                  dictionary["/BitsPerComponent"] as? Int == 8,
                  dictionary["/ImageMask"] == nil, dictionary["/Mask"] == nil, dictionary["/SMask"] == nil,
                  dictionary["/Decode"] == nil, dictionary["/DecodeParms"] == nil,
                  dictionary["/Alternates"] == nil, dictionary["/OPI"] == nil,
                  let color = dictionary["/ColorSpace"] as? String,
                  color == "/DeviceRGB" || color == "/DeviceGray" else { continue }
            // A single, known filter only. No inferred colorspaces or color conversion.
            let filter: String?
            if let single = dictionary["/Filter"] as? String { filter = single }
            else if let array = dictionary["/Filter"] as? [String], array.count == 1 { filter = array.first }
            else { filter = nil }
            guard filter == "/DCTDecode" || filter == "/FlateDecode" || dictionary["/Filter"] == nil else { continue }
            guard totalPixels <= maximumTotalPixels - w * h else { continue }
            totalPixels += w * h
            // Keep aspect ratio and never upsample. All observed occurrences contribute.
            let horizontalRatio: Double = required.width * Double(dpi) / 72.0 / Double(w)
            let verticalRatio: Double = required.height * Double(dpi) / 72.0 / Double(h)
            let ratio: Double = min(1.0, max(horizontalRatio, verticalRatio))
            let width: Int = min(w, max(1, Int(ceil(Double(w) * ratio - 1e-8))))
            let height: Int = min(h, max(1, Int(ceil(Double(h) * ratio - 1e-8))))
            let objectID = reference.replacingOccurrences(of: " R", with: "").replacingOccurrences(of: " ", with: ",")
            let raw = job.appendingPathComponent("image-\(replacements.count)-source")
            let encoded = job.appendingPathComponent("image-\(replacements.count).jpg")
            defer { try? FileManager.default.removeItem(at: raw) }
            _ = try run([input.path, "--show-object=\(objectID)", "--raw-stream-data"], raw, maximumStreamBytes)
            let originalBytes = try raw.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
            guard originalBytes > 0 else { continue }
            let changed = try autoreleasepool { () throws -> Bool in
                if cancelled() { throw OrganizeError.cancelled }
                let image: CGImage
                if filter == "/DCTDecode" {
                    guard let source = CGImageSourceCreateWithURL(raw as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
                          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
                          properties[kCGImagePropertyPixelWidth] as? Int == w,
                          properties[kCGImagePropertyPixelHeight] as? Int == h,
                          let decoded = CGImageSourceCreateImageAtIndex(source, 0, [kCGImageSourceShouldCache: false] as CFDictionary),
                          let decodedSpace = decoded.colorSpace,
                          let standardSpace = CGColorSpace(name: color == "/DeviceRGB" ? CGColorSpace.sRGB : CGColorSpace.genericGrayGamma2_2),
                          CFEqual(decodedSpace, standardSpace) else { return false }
                    // Compare the actual profile, never its user-editable display name.
                    // Wide-gamut/custom ICC JPEGs keep their original stream and interpretation.
                    image = decoded
                } else {
                    let channels = color == "/DeviceRGB" ? 3 : 1
                    _ = try run([input.path, "--show-object=\(objectID)", "--filtered-stream-data"], raw, min(maximumStreamBytes, w * h * channels))
                    let bytes = try Data(contentsOf: raw)
                    guard bytes.count == w * h * channels, let provider = CGDataProvider(data: bytes as CFData),
                          let decoded = CGImage(width: w, height: h, bitsPerComponent: 8, bitsPerPixel: channels * 8,
                            bytesPerRow: w * channels, space: color == "/DeviceRGB" ? CGColorSpaceCreateDeviceRGB() : CGColorSpaceCreateDeviceGray(),
                            bitmapInfo: CGBitmapInfo(rawValue: 0), provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent) else { return false }
                    image = decoded
                }
                if cancelled() { throw OrganizeError.cancelled }
                let target: CGImage
                if width == w && height == h { target = image }
                else {
                    let gray = color == "/DeviceGray"
                    guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                        space: image.colorSpace ?? (gray ? CGColorSpaceCreateDeviceGray() : CGColorSpaceCreateDeviceRGB()),
                        bitmapInfo: gray ? CGImageAlphaInfo.none.rawValue : CGImageAlphaInfo.noneSkipLast.rawValue) else { return false }
                    context.interpolationQuality = .high
                    context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
                    guard let resized = context.makeImage() else { return false }
                    target = resized
                }
                guard let destination = CGImageDestinationCreateWithURL(encoded as CFURL, UTType.jpeg.identifier as CFString, 1, nil) else { return false }
                CGImageDestinationAddImage(destination, target, [kCGImageDestinationLossyCompressionQuality: Double(quality) / 100] as CFDictionary)
                guard CGImageDestinationFinalize(destination) else { return false }
                return true
            }
            if cancelled() { throw OrganizeError.cancelled }
            guard changed, let size = try encoded.resourceValues(forKeys: [.fileSizeKey]).fileSize,
                  size < originalBytes else { try? FileManager.default.removeItem(at: encoded); continue }
            var updated = dictionary
            updated.removeValue(forKey: "/Length")
            updated["/Width"] = width; updated["/Height"] = height; updated["/Filter"] = "/DCTDecode"
            replacements["obj:" + reference] = ["stream": ["dict": updated, "datafile": encoded.path]]
        }
        guard !replacements.isEmpty else { return nil }
        let update = job.appendingPathComponent("image-update.json")
        let data = try JSONSerialization.data(withJSONObject: ["qpdf": [header, replacements]], options: [.sortedKeys])
        guard data.count <= 16 * 1024 * 1024 else { throw OrganizeError.invalidInput }
        guard FileManager.default.createFile(atPath: update.path, contents: data, attributes: [.posixPermissions: 0o600]) else { throw OrganizeError.invalidInput }
        return update
    }
}

private struct ImageMatrix {
    var a: Double = 1, b: Double = 0, c: Double = 0, d: Double = 1, x: Double = 0, y: Double = 0
    static let identity = ImageMatrix()
    var valid: Bool { [a,b,c,d,x,y].allSatisfy { $0.isFinite && abs($0) <= 1e9 } }
    func multiplied(_ m: ImageMatrix) -> ImageMatrix {
        ImageMatrix(a: a*m.a+c*m.b, b: b*m.a+d*m.b, c: a*m.c+c*m.d, d: b*m.c+d*m.d,
                    x: a*m.x+c*m.y+x, y: b*m.x+d*m.y+y)
    }
}

private final class ImageUsage {
    let objects: [String: Any]
    let cancelled: () -> Bool
    var failed = false
    var sizes: [String: (width: Double, height: Double)] = [:]
    var excludedImages: Set<String> = []
    var operations = 0
    let table: CGPDFOperatorTableRef?
    init(objects: [String: Any], cancelled: @escaping () -> Bool) {
        self.objects = objects; self.cancelled = cancelled
        table = CGPDFOperatorTableCreate()
        guard let table else { failed = true; return }
        CGPDFOperatorTableSetCallback(table, "q") { scanner, info in ScanContext.get(info).save(scanner) }
        CGPDFOperatorTableSetCallback(table, "Q") { scanner, info in ScanContext.get(info).restore(scanner) }
        CGPDFOperatorTableSetCallback(table, "cm") { scanner, info in ScanContext.get(info).concatenate(scanner) }
        CGPDFOperatorTableSetCallback(table, "Do") { scanner, info in ScanContext.get(info).paint(scanner) }
        CGPDFOperatorTableSetCallback(table, "BI") { scanner, info in ScanContext.get(info).stop(scanner) }
        // Count/cancel ordinary text and path work too, not only image operators.
        for name in ["b","B","b*","B*","BDC","BMC","BT","c","CS","cs","d","d0","d1","DP","EMC","ET","EX","f","F","f*","G","g","gs","h","i","ID","EI","j","J","K","k","l","m","M","MP","n","re","RG","rg","ri","s","S","SC","sc","SCN","scn","sh","T*","Tc","Td","TD","Tf","Tj","TJ","TL","Tm","Tr","Ts","Tw","Tz","v","w","W","W*","y","'","\"","BX"] {
            CGPDFOperatorTableSetCallback(table, name) { scanner, info in _ = ScanContext.get(info).tick(scanner) }
        }
    }
    deinit { if let table { CGPDFOperatorTableRelease(table) } }
    func dictionary(_ value: Any, depth: Int = 0) -> [String: Any]? {
        guard depth < 32 else { return nil }
        if let reference = value as? String, let object = objects["obj:" + reference] as? [String: Any] {
            if let stream = object["stream"] as? [String: Any] { return stream["dict"] as? [String: Any] }
            if let value = object["value"] { return dictionary(value, depth: depth + 1) }
        }
        return value as? [String: Any]
    }
    func pageResources(_ page: [String: Any], depth: Int = 0) -> [String: Any]? {
        guard depth < 32 else { return nil }
        if let value = page["/Resources"] { return dictionary(value) }
        if let parent = page["/Parent"], let dictionary = dictionary(parent) { return pageResources(dictionary, depth: depth + 1) }
        return [:]
    }
    func excludeUnsupportedImages() {
        var visited: Set<String> = []
        func exclude(_ value: Any, depth: Int = 0) {
            guard depth < 64, !failed, !cancelled() else { failed = true; return }
            if let reference = value as? String, let object = objects["obj:" + reference], visited.insert(reference).inserted {
                if dictionary(reference)?["/Subtype"] as? String == "/Image" { excludedImages.insert(reference) }
                exclude(object, depth: depth + 1)
            } else if let d = value as? [String: Any] {
                for child in d.values { exclude(child, depth: depth + 1) }
            } else if let a = value as? [Any] {
                for child in a { exclude(child, depth: depth + 1) }
            }
        }
        func findUnsupported(_ value: Any, depth: Int = 0) {
            guard depth < 64, !failed, !cancelled() else { failed = true; return }
            if let d = value as? [String: Any] {
                // Pattern/Type3/annotation appearance images may share an image with pages.
                // Do not infer their physical CTM from a different occurrence.
                if d["/PatternType"] != nil || d["/Subtype"] as? String == "/Type3" { exclude(d) }
                for key in ["/AP", "/SMask", "/Mask", "/Alternates", "/OPI"] {
                    if let painting = d[key] { exclude(painting) }
                }
                for child in d.values { findUnsupported(child, depth: depth + 1) }
            } else if let a = value as? [Any] {
                for child in a { findUnsupported(child, depth: depth + 1) }
            }
        }
        findUnsupported(objects)
    }
    func scan(stream: CGPDFContentStreamRef, resources: [String: Any], matrix: ImageMatrix, path: Set<String>,
              inheritedDefaultRGB: Bool = false, inheritedDefaultGray: Bool = false) {
        guard !failed, path.count <= 16, matrix.valid, let table else { failed = true; return }
        let colorSpaces = dictionary(resources["/ColorSpace"] ?? [:])
        // An unresolved resource dictionary is not evidence of ordinary device colors.
        let context = ScanContext(owner: self, resources: resources, matrix: matrix, path: path,
            defaultRGB: inheritedDefaultRGB || colorSpaces == nil || colorSpaces?["/DefaultRGB"] != nil,
            defaultGray: inheritedDefaultGray || colorSpaces == nil || colorSpaces?["/DefaultGray"] != nil)
        let scanner = CGPDFScannerCreate(stream, table, Unmanaged.passUnretained(context).toOpaque())
        defer { CGPDFScannerRelease(scanner) }
        if !CGPDFScannerScan(scanner) || !context.stack.isEmpty { failed = true }
    }
}

private final class ScanContext {
    let owner: ImageUsage
    let resources: [String: Any]
    let path: Set<String>
    let defaultRGB: Bool, defaultGray: Bool
    var matrix: ImageMatrix
    var stack: [ImageMatrix] = []
    init(owner: ImageUsage, resources: [String: Any], matrix: ImageMatrix, path: Set<String>, defaultRGB: Bool, defaultGray: Bool) {
        self.owner = owner; self.resources = resources; self.matrix = matrix; self.path = path
        self.defaultRGB = defaultRGB; self.defaultGray = defaultGray
    }
    static func get(_ info: UnsafeMutableRawPointer?) -> ScanContext { Unmanaged<ScanContext>.fromOpaque(info!).takeUnretainedValue() }
    func stop(_ scanner: CGPDFScannerRef) { owner.failed = true; CGPDFScannerStop(scanner) }
    func tick(_ scanner: CGPDFScannerRef) -> Bool {
        owner.operations += 1
        if owner.failed || owner.cancelled() || owner.operations > 100_000 { stop(scanner); return false }
        return true
    }
    func save(_ scanner: CGPDFScannerRef) {
        guard tick(scanner), stack.count < 256 else { stop(scanner); return }
        stack.append(matrix)
    }
    func restore(_ scanner: CGPDFScannerRef) {
        guard tick(scanner), let saved = stack.popLast() else { stop(scanner); return }
        matrix = saved
    }
    func concatenate(_ scanner: CGPDFScannerRef) {
        guard tick(scanner) else { return }
        var values = [Double](repeating: 0, count: 6)
        for index in (0..<6).reversed() {
            var number: CGPDFReal = 0
            guard CGPDFScannerPopNumber(scanner, &number), number.isFinite else { stop(scanner); return }
            values[index] = Double(number)
        }
        matrix = matrix.multiplied(ImageMatrix(a: values[0], b: values[1], c: values[2], d: values[3], x: values[4], y: values[5]))
        if !matrix.valid { stop(scanner) }
    }
    func paint(_ scanner: CGPDFScannerRef) {
        guard tick(scanner) else { return }
        var pointer: UnsafePointer<CChar>?
        guard CGPDFScannerPopName(scanner, &pointer), let pointer,
              let xobjects = owner.dictionary(resources["/XObject"] ?? [:]),
              let reference = xobjects["/" + String(cString: pointer)] as? String,
              let object = owner.dictionary(reference) else { stop(scanner); return }
        if object["/Subtype"] as? String == "/Image" {
            if (defaultRGB && object["/ColorSpace"] as? String == "/DeviceRGB") ||
               (defaultGray && object["/ColorSpace"] as? String == "/DeviceGray") {
                // Exclusion is global: any overridden occurrence protects a shared stream.
                owner.excludedImages.insert(reference)
            }
            let width = hypot(matrix.a, matrix.b), height = hypot(matrix.c, matrix.d)
            guard width > 0, height > 0, width.isFinite, height.isFinite else { stop(scanner); return }
            let old = owner.sizes[reference] ?? (0, 0)
            owner.sizes[reference] = (max(old.width, width), max(old.height, height))
        } else if object["/Subtype"] as? String == "/Form" {
            guard !path.contains(reference), path.count < 16,
                  object["/Group"] == nil, object["/Ref"] == nil, object["/OPI"] == nil,
                  let resourceObject = CGPDFContentStreamGetResource(CGPDFScannerGetContentStream(scanner), "XObject", pointer) else { stop(scanner); return }
            var stream: CGPDFStreamRef?
            guard CGPDFObjectGetValue(resourceObject, .stream, &stream), let stream,
                  let nativeDictionary = CGPDFStreamGetDictionary(stream) else { stop(scanner); return }
            var nativeResources: CGPDFDictionaryRef?
            guard CGPDFDictionaryGetDictionary(nativeDictionary, "Resources", &nativeResources), let nativeResources,
                  let resources = owner.dictionary(object["/Resources"] ?? [:]) else { stop(scanner); return }
            let child = CGPDFContentStreamCreateWithStream(stream, nativeResources, CGPDFScannerGetContentStream(scanner))
            defer { CGPDFContentStreamRelease(child) }
            var formMatrix = ImageMatrix.identity
            if let raw = object["/Matrix"] {
                guard let values = raw as? [Double], values.count == 6, values.allSatisfy(\.isFinite) else { stop(scanner); return }
                formMatrix = ImageMatrix(a: values[0], b: values[1], c: values[2], d: values[3], x: values[4], y: values[5])
            }
            owner.scan(stream: child, resources: resources, matrix: matrix.multiplied(formMatrix), path: path.union([reference]),
                       inheritedDefaultRGB: defaultRGB, inheritedDefaultGray: defaultGray)
            if owner.failed { stop(scanner) }
        } else { stop(scanner) }
    }
}
