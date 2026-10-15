import XCTest
import AppKit
import PDFKit
@testable import PaperCore

final class ImageImportIntegrationTests: XCTestCase {
    func testImageImportProducesTwoRealPDFPagesOffThread() async throws {
        let fixtures = FileManager.default.temporaryDirectory.appendingPathComponent("ImageImportFixture-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: fixtures, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: fixtures) }
        let images = [fixtures.appendingPathComponent("red landscape.png"), fixtures.appendingPathComponent("blue portrait.jpg")]
        for (index, url) in images.enumerated() {
            let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: index == 0 ? 160 : 80,
                pixelsHigh: index == 0 ? 80 : 160, bitsPerSample: 8, samplesPerPixel: 3,
                hasAlpha: false, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
            for x in 0..<bitmap.pixelsWide { for y in 0..<bitmap.pixelsHigh {
                bitmap.setColor(index == 0 ? .red : .blue, atX: x, y: y)
            } }
            try XCTUnwrap(bitmap.representation(using: index == 0 ? .png : .jpeg, properties: [:])).write(to: url)
        }
        let result = try await LocalToolJob.run(data: Data(), cancellation: LocalToolCancellation()) { _, output, flag in
            let count = try ImagePDFImport.convert(images, to: output, cancelled: { flag.isCancelled })
            return "\(count)"
        }
        XCTAssertEqual(result.message, "2")
        let doc = try XCTUnwrap(PDFDocument(data: result.data))
        XCTAssertEqual(doc.pageCount, 2)
        XCTAssertEqual(doc.page(at: 0)!.bounds(for: .mediaBox).width, 595.276, accuracy: 0.1)
    }
}
