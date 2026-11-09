import XCTest
import AppKit
@testable import PaperCore

final class SignatureImageTests: XCTestCase {
    func testValidPNGNormalizesAndRejectsInvalidData() throws {
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 3, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        let image = try SignatureImage(pngData: rep.representation(using: .png, properties: [:])!)
        XCTAssertEqual(image.pixelWidth, 3)
        XCTAssertEqual(image.pixelHeight, 2)
        XCTAssertThrowsError(try SignatureImage(pngData: Data("not png".utf8)))
        XCTAssertThrowsError(try SignatureImage(pngData: Data(repeating: 0, count: 8 * 1024 * 1024 + 1)))
    }
}
