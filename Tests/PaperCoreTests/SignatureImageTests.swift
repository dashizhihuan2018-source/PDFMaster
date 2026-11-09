import XCTest
import AppKit
@testable import PaperCore

final class SignatureImageTests: XCTestCase {
    private func png(_ width: Int = 3, _ height: Int = 2) -> Data {
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        return rep.representation(using: .png, properties: [:])!
    }

    func testRejectsTruncationCRCHeaderAndAPNG() throws {
        let data = png()
        XCTAssertThrowsError(try SignatureImage(pngData: data.dropLast(12)))
        XCTAssertThrowsError(try SignatureImage(pngData: data.dropLast(24)))
        var crc = data; crc[crc.count - 1] ^= 1
        XCTAssertThrowsError(try SignatureImage(pngData: crc))
        var header = data; header[12] = 88
        XCTAssertThrowsError(try SignatureImage(pngData: header))
        // Valid CRC for an acTL chunk containing one frame and zero plays.
        let actl: [UInt8] = [0,0,0,8,97,99,84,76,0,0,0,1,0,0,0,0,0xb4,0x2d,0xe9,0xa0]
        var animated = data; animated.insert(contentsOf: actl, at: 33)
        XCTAssertThrowsError(try SignatureImage(pngData: animated))
        XCTAssertThrowsError(try SignatureImage(pngData: data + Data([0])))
    }

    func testPixelLimitBoundary() throws {
        XCTAssertEqual(try SignatureImage(pngData: png(2000, 2000)).pixelWidth, 2000)
        XCTAssertThrowsError(try SignatureImage(pngData: png(2001, 2000)))
    }
    func testValidPNGNormalizesAndRejectsInvalidData() throws {
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 3, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        let image = try SignatureImage(pngData: rep.representation(using: .png, properties: [:])!)
        XCTAssertEqual(image.pixelWidth, 3)
        XCTAssertEqual(image.pixelHeight, 2)
        XCTAssertThrowsError(try SignatureImage(pngData: Data("not png".utf8)))
        XCTAssertThrowsError(try SignatureImage(pngData: Data(repeating: 0, count: 8 * 1024 * 1024 + 1)))
    }
}
