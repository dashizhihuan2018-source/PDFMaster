import Foundation
import ImageIO
import UniformTypeIdentifiers

public struct SignatureImage {
    public let pngData: Data
    public let pixelWidth: Int
    public let pixelHeight: Int

    public init(pngData: Data) throws {
        enum Invalid: Error { case image }
        guard pngData.count <= 8 * 1024 * 1024,
              pngData.starts(with: [137, 80, 78, 71, 13, 10, 26, 10]),
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
}
