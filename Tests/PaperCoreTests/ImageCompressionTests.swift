import CoreGraphics
import ImageIO
import XCTest
@testable import PaperCore

final class ImageCompressionTests: XCTestCase {
    func testDirectImageConversionAcceptsNineTwelveMegapixelPhotos() throws {
        let root = try directory()
        let context = try XCTUnwrap(CGContext(data: nil, width: 4000, height: 3000, bitsPerComponent: 8, bytesPerRow: 16000,
                                            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
        context.setFillColor(CGColor(red: 0.2, green: 0.4, blue: 0.8, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 4000, height: 3000))
        let input = root.appendingPathComponent("phone.jpg")
        try write(try XCTUnwrap(context.makeImage()), to: input, type: "public.jpeg")
        let original = try Data(contentsOf: input)
        let output = root.appendingPathComponent("nine-photos.pdf")
        XCTAssertEqual(try ImagePDFImport.convert(Array(repeating: input, count: 9), to: output), 9)
        XCTAssertEqual(CGPDFDocument(output as CFURL)?.numberOfPages, 9)
        XCTAssertEqual(try Data(contentsOf: input), original)
    }

    func testJPEGEmbedsOriginalCompressedSamplesAtFullResolution() throws {
        let root = try directory()
        let input = root.appendingPathComponent("detail.jpg")
        try write(try noiseImage(width: 320, height: 200), to: input, type: "public.jpeg")
        let original = try Data(contentsOf: input)
        let output = root.appendingPathComponent("detail.pdf")
        XCTAssertEqual(try ImagePDFImport.convert([input], to: output, layout: .init(size: nil, margin: 0)), 1)
        let document = try XCTUnwrap(CGPDFDocument(output as CFURL))
        let page = try XCTUnwrap(document.page(at: 1))
        let streams = try imageStreams(page)
        XCTAssertEqual(streams.count, 1)
        let stream = try XCTUnwrap(streams.first)
        let dictionary = try XCTUnwrap(CGPDFStreamGetDictionary(stream))
        var width: CGPDFInteger = 0, height: CGPDFInteger = 0
        XCTAssertTrue(CGPDFDictionaryGetInteger(dictionary, "Width", &width))
        XCTAssertTrue(CGPDFDictionaryGetInteger(dictionary, "Height", &height))
        XCTAssertEqual(width, 320)
        XCTAssertEqual(height, 200)
        var format = CGPDFDataFormat.raw
        let embedded = try XCTUnwrap(CGPDFStreamCopyData(stream, &format)) as Data
        XCTAssertEqual(format, .jpegEncoded, "JPEG must remain an encoded PDF image stream")
        if format == .jpegEncoded {
            // PDF writers may remove EXIF/JFIF markers without changing the
            // compressed image. Compare the entropy-coded samples, not metadata.
            XCTAssertEqual(try scanData(embedded), try scanData(original), "JPEG samples were recompressed")
        }
        XCTAssertLessThan(try Data(contentsOf: output).count, original.count + 32 * 1024)
        XCTAssertEqual(try Data(contentsOf: input), original)
    }

    @MainActor func testJPEGCompressedSamplesSurviveWorkspaceMergeSaveAndReopen() throws {
        let root = try directory()
        let input = root.appendingPathComponent("detail-rotated.jpg")
        try write(try noiseImage(width: 320, height: 200), to: input, type: "public.jpeg", orientation: 6)
        let original = try Data(contentsOf: input)
        let converted = root.appendingPathComponent("converted.pdf")
        XCTAssertEqual(try ImagePDFImport.convert([input], to: converted, layout: .init(size: nil, margin: 0)), 1)
        let data = try Data(contentsOf: converted)
        let workspace = PDFWorkspace()
        try workspace.createUnsavedFileMerge([data, data])
        XCTAssertEqual(workspace.document?.pageCount, 2)
        let saved = root.appendingPathComponent("saved-two-photos.pdf")
        try workspace.saveCopy(to: saved)
        let reopened = try XCTUnwrap(CGPDFDocument(saved as CFURL))
        XCTAssertEqual(reopened.numberOfPages, 2)
        for index in 1...2 {
            let page = try XCTUnwrap(reopened.page(at: index))
            XCTAssertEqual(page.getBoxRect(.mediaBox).size, CGSize(width: 200, height: 320))
            let streams = try imageStreams(page)
            XCTAssertEqual(streams.count, 1)
            let stream = try XCTUnwrap(streams.first)
            let dictionary = try XCTUnwrap(CGPDFStreamGetDictionary(stream))
            var width: CGPDFInteger = 0, height: CGPDFInteger = 0
            XCTAssertTrue(CGPDFDictionaryGetInteger(dictionary, "Width", &width))
            XCTAssertTrue(CGPDFDictionaryGetInteger(dictionary, "Height", &height))
            XCTAssertEqual(width, 320)
            XCTAssertEqual(height, 200)
            var format = CGPDFDataFormat.raw
            let embedded = try XCTUnwrap(CGPDFStreamCopyData(stream, &format)) as Data
            XCTAssertEqual(format, .jpegEncoded)
            if format == .jpegEncoded {
                XCTAssertEqual(try scanData(embedded), try scanData(original), "Saved page \(index) recompressed JPEG samples")
            }
        }
        XCTAssertEqual(try Data(contentsOf: input), original)
    }

    func testAllEightJPEGOrientationsMatchImageIOWithoutMirrorsOrBlankPages() throws {
        let root = try directory()
        let fixture = try colorImage()
        for orientation in 1...8 {
            let input = root.appendingPathComponent("orientation-\(orientation).jpg")
            try write(fixture, to: input, type: "public.jpeg", orientation: orientation)
            let source = try XCTUnwrap(CGImageSourceCreateWithURL(input as CFURL, nil))
            let properties = try XCTUnwrap(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
            XCTAssertEqual((properties[kCGImagePropertyOrientation] as? NSNumber)?.intValue, orientation)
            let oracle = try XCTUnwrap(CGImageSourceCreateThumbnailAtIndex(source, 0, [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceThumbnailMaxPixelSize: 160
            ] as CFDictionary))
            let output = root.appendingPathComponent("orientation-\(orientation).pdf")
            XCTAssertEqual(try ImagePDFImport.convert([input], to: output, layout: .init(size: nil, margin: 0)), 1)
            let pdf = try XCTUnwrap(CGPDFDocument(output as CFURL))
            let page = try XCTUnwrap(pdf.page(at: 1))
            XCTAssertEqual(page.getBoxRect(.mediaBox).size, CGSize(width: oracle.width, height: oracle.height))
            let expected = try bitmap(width: oracle.width, height: oracle.height) { $0.draw(oracle, in: CGRect(x: 0, y: 0, width: oracle.width, height: oracle.height)) }
            let actual = try bitmap(width: oracle.width, height: oracle.height) { $0.drawPDFPage(page) }
            for fy in [0.2, 0.35, 0.65, 0.8] {
                for fx in [0.2, 0.35, 0.65, 0.8] {
                    let offset = (Int(Double(oracle.height) * fy) * oracle.width + Int(Double(oracle.width) * fx)) * 4
                    for channel in 0..<3 {
                        XCTAssertEqual(Double(actual[offset + channel]), Double(expected[offset + channel]), accuracy: 6,
                                       "orientation \(orientation), point \(fx),\(fy), channel \(channel)")
                    }
                }
            }
        }
    }

    func testTransparentPNGStillCompositesOntoWhiteWithoutJPEGArtifacts() throws {
        let root = try directory()
        let width = 64, height = 40
        var bytes = [UInt8](repeating: 0, count: width * height * 4)
        for y in 0..<height { for x in 0..<width where x >= width / 2 {
            let offset = (y * width + x) * 4
            bytes[offset] = 128; bytes[offset + 3] = 128
        } }
        let image = try cgImage(bytes, width: width, height: height, alpha: .premultipliedLast)
        let input = root.appendingPathComponent("transparent.png")
        try write(image, to: input, type: "public.png")
        let original = try Data(contentsOf: input)
        let output = root.appendingPathComponent("transparent.pdf")
        XCTAssertEqual(try ImagePDFImport.convert([input], to: output, layout: .init(size: nil, margin: 0)), 1)
        let pdf = try XCTUnwrap(CGPDFDocument(output as CFURL))
        let page = try XCTUnwrap(pdf.page(at: 1))
        let rendered = try bitmap(width: width, height: height) { $0.drawPDFPage(page) }
        for (x, expected) in [(16, [255.0, 255, 255]), (48, [255.0, 127, 127])] {
            let offset = (20 * width + x) * 4
            for channel in 0..<3 { XCTAssertEqual(Double(rendered[offset + channel]), expected[channel], accuracy: 3) }
        }
        XCTAssertEqual(try Data(contentsOf: input), original)
    }

    private final class Streams { var values: [CGPDFStreamRef] = [] }
    private func imageStreams(_ page: CGPDFPage) throws -> [CGPDFStreamRef] {
        var resources: CGPDFDictionaryRef?, objects: CGPDFDictionaryRef?
        XCTAssertTrue(CGPDFDictionaryGetDictionary(try XCTUnwrap(page.dictionary), "Resources", &resources))
        XCTAssertTrue(CGPDFDictionaryGetDictionary(try XCTUnwrap(resources), "XObject", &objects))
        let streams = Streams()
        CGPDFDictionaryApplyFunction(try XCTUnwrap(objects), { _, object, info in
            var stream: CGPDFStreamRef?
            guard CGPDFObjectGetValue(object, .stream, &stream), let stream,
                  let dictionary = CGPDFStreamGetDictionary(stream), let info else { return }
            var subtype: UnsafePointer<CChar>?
            guard CGPDFDictionaryGetName(dictionary, "Subtype", &subtype), let subtype,
                  String(cString: subtype) == "Image" else { return }
            Unmanaged<Streams>.fromOpaque(info).takeUnretainedValue().values.append(stream)
        }, Unmanaged.passUnretained(streams).toOpaque())
        return streams.values
    }

    private func scanData(_ jpeg: Data) throws -> Data {
        let bytes = [UInt8](jpeg)
        guard bytes.count > 4, bytes[0] == 0xff, bytes[1] == 0xd8 else { throw CocoaError(.fileReadCorruptFile) }
        var offset = 2
        while offset + 4 <= bytes.count {
            guard bytes[offset] == 0xff else { throw CocoaError(.fileReadCorruptFile) }
            let marker = bytes[offset + 1]
            let length = Int(bytes[offset + 2]) * 256 + Int(bytes[offset + 3])
            guard length >= 2, offset + 2 + length <= bytes.count else { throw CocoaError(.fileReadCorruptFile) }
            if marker == 0xda {
                guard bytes[bytes.count - 2] == 0xff, bytes.last == 0xd9 else { throw CocoaError(.fileReadCorruptFile) }
                return Data(bytes[(offset + 2 + length)..<(bytes.count - 2)])
            }
            offset += 2 + length
        }
        throw CocoaError(.fileReadCorruptFile)
    }

    private func directory() throws -> URL {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ImageCompressionTests-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        return root
    }
    private func write(_ image: CGImage, to url: URL, type: String, orientation: Int = 1) throws {
        let writer = try XCTUnwrap(CGImageDestinationCreateWithURL(url as CFURL, type as CFString, 1, nil))
        CGImageDestinationAddImage(writer, image, [kCGImageDestinationLossyCompressionQuality: 0.72, kCGImagePropertyOrientation: orientation] as CFDictionary)
        XCTAssertTrue(CGImageDestinationFinalize(writer))
    }
    private func cgImage(_ bytes: [UInt8], width: Int, height: Int, alpha: CGImageAlphaInfo) throws -> CGImage {
        let provider = try XCTUnwrap(CGDataProvider(data: Data(bytes) as CFData))
        return try XCTUnwrap(CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
                                    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGBitmapInfo(rawValue: alpha.rawValue),
                                    provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent))
    }
    private func noiseImage(width: Int, height: Int) throws -> CGImage {
        var state: UInt32 = 0x12345678
        var bytes = [UInt8](repeating: 255, count: width * height * 4)
        for pixel in 0..<(width * height) { for channel in 0..<3 {
            state = state &* 1664525 &+ 1013904223
            bytes[pixel * 4 + channel] = UInt8(truncatingIfNeeded: state >> 24)
        } }
        return try cgImage(bytes, width: width, height: height, alpha: .noneSkipLast)
    }
    private func colorImage() throws -> CGImage {
        let colors: [[UInt8]] = [[240, 25, 30], [30, 220, 40], [25, 40, 235], [235, 210, 25]]
        var bytes = [UInt8](repeating: 255, count: 160 * 96 * 4)
        for y in 0..<96 { for x in 0..<160 {
            let color = colors[(y < 48 ? 0 : 2) + (x < 80 ? 0 : 1)]
            for channel in 0..<3 { bytes[(y * 160 + x) * 4 + channel] = color[channel] }
        } }
        return try cgImage(bytes, width: 160, height: 96, alpha: .noneSkipLast)
    }
    private func bitmap(width: Int, height: Int, draw: (CGContext) -> Void) throws -> [UInt8] {
        let context = try XCTUnwrap(CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                                            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.setFillColor(CGColor(gray: 1, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        draw(context)
        context.flush()
        return Array(UnsafeBufferPointer(start: try XCTUnwrap(context.data).assumingMemoryBound(to: UInt8.self), count: width * height * 4))
    }
}
