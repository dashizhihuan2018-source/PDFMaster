import Foundation
import PDFKit
import XCTest
@testable import PaperCore

final class NativeContentStyleTests: XCTestCase {
    private lazy var publicFixture = try! PublicContentFixture()
    private var source: URL { publicFixture.source }
    private var engine: ContentEditEngine { ContentEditEngine() }

    func testExplicitStylePreservesDecimalSizeColorAndRealSearchableContent() throws {
        let original = try Data(contentsOf: source)
        let page = try engine.inspect(source, page: 0, cancelled: { false })
        let target = try XCTUnwrap(page.spans.first { $0.text == "Original target" })
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("NativeStyle-\(UUID())")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: dir) }
        let input = dir.appendingPathComponent("input.pdf"), output = dir.appendingPathComponent("output.pdf")
        try original.write(to: input)
        var request = ContentEditRequest(operation: "replace-text", page: 0)
        request.id = target.id; request.text = target.text
        request.textStyle = ContentTextStyle(fontName: "Times-Bold", size: 10.5, color: 0x3366CC)
        _ = try engine.apply(input, request: request, output: output, cancelled: { false })
        let result = try engine.inspect(output, page: 0, cancelled: { false })
        let edited = try XCTUnwrap(result.spans.first { $0.text == target.text })
        XCTAssertTrue(edited.font.contains("Times-Bold"))
        XCTAssertEqual(edited.size, 10.5, accuracy: 0.01)
        XCTAssertEqual(edited.color, 0x3366CC)
        XCTAssertEqual(result.spans.filter { $0.text == "Neighbor preserved" }.map(\.text), ["Neighbor preserved"])
        let document = try XCTUnwrap(PDFDocument(url: output))
        XCTAssertTrue(document.page(at: 0)?.string?.contains(target.text) == true)
        XCTAssertFalse(document.page(at: 0)!.annotations.contains { ["FreeText", "Square", "Redact"].contains($0.type ?? "") })
        XCTAssertEqual(try Data(contentsOf: source), original)
        XCTAssertEqual(try Data(contentsOf: input), original)
    }

    func testInvalidAndUnavailableExplicitStylesAreRejectedWithoutOutput() throws {
        let page = try engine.inspect(source, page: 0, cancelled: { false })
        let span = try XCTUnwrap(page.spans.first { $0.text == "Original target" })
        let styles = [ContentTextStyle(fontName: "Times-Roman", size: 0, color: 0),
                      ContentTextStyle(fontName: "Times-Roman", size: .infinity, color: 0),
                      ContentTextStyle(fontName: "Times-Roman", size: .nan, color: 0),
                      ContentTextStyle(fontName: "Times-Roman", size: 513, color: 0),
                      ContentTextStyle(fontName: "Times-Roman", size: 14, color: -1),
                      ContentTextStyle(fontName: "Times-Roman", size: 14, color: 0x1000000),
                      ContentTextStyle(fontName: "", size: 14, color: 0),
                      ContentTextStyle(fontName: "NoSuchPDFMasterFont-\(UUID())", size: 14, color: 0)]
        for style in styles {
            XCTAssertThrowsError(try NativeContentTextPDF.render(span.text, span: span, page: page, style: style))
        }
    }

    func testLargeStyleStillRespectsOriginalFootprintAndNeverRewritesSource() throws {
        let original = try Data(contentsOf: source)
        let span = try XCTUnwrap(engine.inspect(source, page: 0, cancelled: { false }).spans.first { $0.text == "Original target" })
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("NativeStyle-overflow-\(UUID())")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: dir) }
        let input = dir.appendingPathComponent("input.pdf"), output = dir.appendingPathComponent("output.pdf")
        try original.write(to: input)
        var request = ContentEditRequest(operation: "replace-text", page: 0)
        request.id = span.id; request.text = span.text
        request.textStyle = ContentTextStyle(fontName: "Courier-Bold", size: 96, color: 0xFF0000)
        XCTAssertThrowsError(try engine.apply(input, request: request, output: output, cancelled: { false }))
        XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
        XCTAssertEqual(try Data(contentsOf: input), original)
        XCTAssertEqual(try Data(contentsOf: source), original)
    }

    func testOriginalFontFallbackCannotBeUsedForAnUnrelatedExplicitName() throws {
        let page = try engine.inspect(source, page: 0, cancelled: { false })
        let span = try XCTUnwrap(page.spans.first { $0.text == "Original target" })
        let style = ContentTextStyle(fontName: "NoSuchFont-\(UUID())", size: 14, color: 0, retainsOriginalFont: true)
        XCTAssertThrowsError(try NativeContentTextPDF.render(span.text, span: span, page: page, style: style))
        let legacy = Data(#"{"fontName":"Times-Roman","size":14,"color":0}"#.utf8)
        XCTAssertNil(try JSONDecoder().decode(ContentTextStyle.self, from: legacy).retainsOriginalFont)
    }
}
