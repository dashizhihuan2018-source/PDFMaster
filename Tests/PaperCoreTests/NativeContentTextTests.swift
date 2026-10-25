import Foundation
import PDFKit
import XCTest
@testable import PaperCore

final class NativeContentTextTests: XCTestCase {
    private lazy var publicFixture = try! PublicContentFixture()
    func testReplacementUsesNativeGlyphLayoutAndSearchablePDFResources() throws {
        let source = publicFixture.source
        let original = try Data(contentsOf: source)
        let engine = ContentEditEngine()
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("NativeContentText-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: dir) }
        let input = dir.appendingPathComponent("input.pdf"), output = dir.appendingPathComponent("edited.pdf")
        try original.write(to: input)
        let span = try XCTUnwrap(engine.inspect(input, page: 0, cancelled: { false }).spans.first { $0.text == "Original target" })
        var request = ContentEditRequest(operation: "replace-text", page: 0)
        request.id = span.id; request.text = "Edited Ω"
        let message = try engine.apply(input, request: request, output: output, cancelled: { false })
        let doc = try XCTUnwrap(PDFDocument(url: output))
        XCTAssertTrue(doc.page(at: 0)?.string?.contains("Edited Ω") == true)
        XCTAssertFalse(doc.page(at: 0)?.string?.contains("Original target") == true)
        XCTAssertEqual(doc.pageCount, PDFDocument(data: original)?.pageCount)
        XCTAssertTrue(message.contains("原生排版"))
        XCTAssertEqual(try Data(contentsOf: source), original)
        XCTAssertEqual(try Data(contentsOf: input), original)
    }
}
