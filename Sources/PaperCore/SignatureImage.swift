import Foundation
import ImageIO
import UniformTypeIdentifiers

public struct SignatureImage {
    private enum Invalid: LocalizedError {
        case image
        var errorDescription: String? { "PNG 图像无效、损坏或超出限制。" }
    }
    public let pngData: Data
    public let pixelWidth: Int
    public let pixelHeight: Int

    public init(pngData: Data) throws {
        guard pngData.count <= 8 * 1024 * 1024,
              Self.validPNG(pngData),
              let source = CGImageSourceCreateWithData(pngData as CFData, nil),
              CGImageSourceGetType(source) == UTType.png.identifier as CFString,
              CGImageSourceGetCount(source) == 1,
              CGImageSourceGetStatus(source) == .statusComplete,
              CGImageSourceGetStatusAtIndex(source, 0) == .statusComplete,
              let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = props[kCGImagePropertyPixelWidth] as? Int,
              let height = props[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, width <= 4_000_000 / height,
              let image = CGImageSourceCreateImageAtIndex(source, 0, [kCGImageSourceShouldCacheImmediately: true] as CFDictionary)
        else { throw Invalid.image }
        // Draw into a fresh RGBA surface so source metadata is never copied.
        guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
                                      bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue),
              let output = CFDataCreateMutable(nil, 0) else { throw Invalid.image }
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        guard let normalized = context.makeImage(),
              let destination = CGImageDestinationCreateWithData(output, UTType.png.identifier as CFString, 1, nil)
        else { throw Invalid.image }
        CGImageDestinationAddImage(destination, normalized, nil)
        guard CGImageDestinationFinalize(destination), CFDataGetLength(output) <= 8 * 1024 * 1024 else { throw Invalid.image }
        self.pngData = output as Data
        self.pixelWidth = width
        self.pixelHeight = height
    }

    // ImageIO can decode incomplete PNGs; validate framing and CRC first.
    private static func validPNG(_ data: Data) -> Bool {
        let b = [UInt8](data)
        guard b.count >= 8, b.starts(with: [137, 80, 78, 71, 13, 10, 26, 10]) else { return false }
        func word(_ i: Int) -> UInt32 {
            UInt32(b[i]) << 24 | UInt32(b[i + 1]) << 16 | UInt32(b[i + 2]) << 8 | UInt32(b[i + 3])
        }
        func crc(_ range: Range<Int>) -> UInt32 {
            var c: UInt32 = 0xffffffff
            for byte in b[range] {
                c ^= UInt32(byte)
                for _ in 0..<8 { c = (c >> 1) ^ (c & 1 == 1 ? 0xedb88320 : 0) }
            }
            return c ^ 0xffffffff
        }
        var offset = 8, seenHeader = false, seenData = false, endedData = false, seenPalette = false
        while offset <= b.count - 12 {
            let length = Int(word(offset))
            guard length <= b.count - offset - 12 else { return false }
            let typeStart = offset + 4, end = offset + 8 + length
            let type = Array(b[typeStart..<(typeStart + 4)])
            guard type.allSatisfy({ (65...90).contains($0) || (97...122).contains($0) }),
                  type[2] & 32 == 0, crc(typeStart..<end) == word(end) else { return false }
            let name = String(decoding: type, as: UTF8.self)
            if !seenHeader {
                guard name == "IHDR", length == 13 else { return false }
                let w = Int(word(offset + 8)), h = Int(word(offset + 12))
                guard w > 0, h > 0, w <= 4_000_000 / h else { return false }
                seenHeader = true
            } else if name == "IHDR" || name == "acTL" || name == "fcTL" || name == "fdAT" {
                return false
            } else if name == "IDAT" {
                guard !endedData else { return false }
                seenData = true
            } else if name == "IEND" {
                return length == 0 && seenData && end + 4 == b.count
            } else {
                if seenData { endedData = true }
                if name == "PLTE" {
                    guard !seenPalette, !seenData, length > 0, length <= 768, length % 3 == 0 else { return false }
                    seenPalette = true
                } else if type[0] & 32 == 0 { return false }
            }
            offset = end + 4
        }
        return false
    }
}
