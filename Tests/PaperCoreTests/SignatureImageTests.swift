import XCTest
import AppKit
import zlib
@testable import PaperCore

final class SignatureImageTests: XCTestCase {
    private func chunk(_ name: String, _ payload: [UInt8]) -> [UInt8] {
        let type = Array(name.utf8)
        let n = UInt32(payload.count)
        let content = type + payload
        let checksum = content.withUnsafeBytes { crc32(0, $0.bindMemory(to: Bytef.self).baseAddress, uInt(content.count)) }
        return [UInt8(n >> 24), UInt8((n >> 16) & 255), UInt8((n >> 8) & 255), UInt8(n & 255)] + content + [UInt8((checksum >> 24) & 255), UInt8((checksum >> 16) & 255), UInt8((checksum >> 8) & 255), UInt8(checksum & 255)]
    }
    private func replacingIDAT(_ data: Data, change: ([UInt8]) -> [UInt8]) -> Data {
        let b = [UInt8](data)
        var pos = 8, result = Array(b[0..<8]), all: [UInt8] = [], inserted = false
        while pos < b.count {
            let n = Int(b[pos]) << 24 | Int(b[pos+1]) << 16 | Int(b[pos+2]) << 8 | Int(b[pos+3])
            let name = String(decoding: b[(pos+4)..<(pos+8)], as: UTF8.self)
            if name == "IDAT" { all += b[(pos+8)..<(pos+8+n)] }
            else {
                if !all.isEmpty && !inserted { result += chunk("IDAT", change(all)); inserted = true }
                result += b[pos..<(pos+12+n)]
            }
            pos += n + 12
        }
        return Data(result)
    }
    private func deflate(_ bytes: [UInt8]) -> [UInt8] {
        var size = compressBound(uLong(bytes.count))
        var out = [UInt8](repeating: 0, count: Int(size))
        let result = bytes.withUnsafeBufferPointer { input in out.withUnsafeMutableBufferPointer { output in compress2(output.baseAddress, &size, input.baseAddress, uLong(bytes.count), Z_DEFAULT_COMPRESSION) } }
        precondition(result == Z_OK)
        return Array(out.prefix(Int(size)))
    }
    func testRejectsIncompleteZlibBadAdlerAndWrongScanlineSize() {
        let data = png()
        for n in 1...5 { XCTAssertThrowsError(try SignatureImage(pngData: replacingIDAT(data) { Array($0.dropLast(n)) })) }
        XCTAssertThrowsError(try SignatureImage(pngData: replacingIDAT(data) { var b = $0; b[b.count - 1] ^= 1; return b }))
        XCTAssertThrowsError(try SignatureImage(pngData: replacingIDAT(data) { _ in deflate([0, 1, 2, 3]) }))
        XCTAssertThrowsError(try SignatureImage(pngData: replacingIDAT(data) { _ in deflate([UInt8](repeating: 0, count: 1000)) }))
        XCTAssertThrowsError(try SignatureImage(pngData: replacingIDAT(data) { $0 + [0] }))
        var badFilter = [UInt8](repeating: 0, count: 26); badFilter[0] = 5
        XCTAssertThrowsError(try SignatureImage(pngData: replacingIDAT(data) { _ in deflate(badFilter) }))
    }
    func testValidAdam7Interlace() throws {
        let header: [UInt8] = [0,0,0,3,0,0,0,2,8,6,0,0,1]
        let signature: [UInt8] = [137,80,78,71,13,10,26,10]
        // Four non-empty Adam7 pass rows: 1, 1, 1 and 3 RGBA pixels.
        let rows = [UInt8](repeating: 0, count: 28)
        let data = Data(signature + chunk("IHDR", header) + chunk("IDAT", deflate(rows)) + chunk("IEND", []))
        let result = try SignatureImage(pngData: data)
        XCTAssertEqual(result.pixelWidth, 3)
        XCTAssertEqual(result.pixelHeight, 2)
    }
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
