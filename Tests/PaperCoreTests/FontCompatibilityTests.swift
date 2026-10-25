import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor
final class FontCompatibilityTests: XCTestCase {
    func testMalformedEmbeddedCFFRendersLikeCorrectTrueTypeAndSurvivesSave() throws {
        let env = ProcessInfo.processInfo.environment
        let generated = FileManager.default.temporaryDirectory.appendingPathComponent("FontFixture-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: generated) }
        var source = env["PDFMASTER_FONT_FIXTURE"], reference = env["PDFMASTER_FONT_REFERENCE"]
        if source == nil || reference == nil {
            try FileManager.default.createDirectory(at: generated, withIntermediateDirectories: false)
            let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            let task = Process(); task.executableURL = ContentEditEngine.localPython
            task.arguments = ["-B", "-I", "-S", root.appendingPathComponent("Tests/ContentEditEngineTests/font_fixture.py").path, generated.path]
            try task.run(); task.waitUntilExit()
            guard task.terminationStatus == 0 else { throw XCTSkip("External local font compatibility test runtime unavailable") }
            source = generated.appendingPathComponent("broken.pdf").path
            reference = generated.appendingPathComponent("reference.pdf").path
        }
        let sourceURL = URL(fileURLWithPath: try XCTUnwrap(source))
        let original = try Data(contentsOf: sourceURL)
        let expected = try XCTUnwrap(PDFDocument(url: URL(fileURLWithPath: try XCTUnwrap(reference))))
        let workspace = PDFWorkspace()
        try workspace.open(sourceURL)
        let actual = try XCTUnwrap(workspace.document)
        XCTAssertEqual(actual.pageCount, expected.pageCount)
        let index = Int(env["PDFMASTER_FONT_PAGE"] ?? "0")!
        let referencePixels = try pixels(expected, page: index)
        XCTAssertTrue(PDFFontCompatibility.needsRepair(original))
        XCTAssertFalse(PDFFontCompatibility.needsRepair(try XCTUnwrap(expected.dataRepresentation())))
        XCTAssertGreaterThan(try difference(pixels(try XCTUnwrap(PDFDocument(data: original)), page: index), referencePixels), 0.001,
                             "The unmodified input must actually reproduce the rendering defect")
        XCTAssertLessThan(try difference(pixels(actual, page: index), referencePixels), 0.001,
                          "Opening must repair actual glyph rendering, not just text extraction")
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: dir) }
        let saved = dir.appendingPathComponent("copy.pdf")
        try workspace.saveCopy(to: saved)
        let reopened = try XCTUnwrap(PDFDocument(url: saved))
        XCTAssertLessThan(try difference(pixels(reopened, page: index), referencePixels), 0.001)
        XCTAssertEqual(try Data(contentsOf: sourceURL), original)
        XCTAssertTrue(actual.page(at: index)?.string?.contains(env["PDFMASTER_FONT_TEXT"] ?? "AB") == true)
    }

    private func pixels(_ document: PDFDocument, page: Int) throws -> Data {
        let image = try XCTUnwrap(document.page(at: page)).thumbnail(of: NSSize(width: 1200, height: 800), for: .mediaBox)
        let rep = try XCTUnwrap(NSBitmapImageRep(data: try XCTUnwrap(image.tiffRepresentation)))
        return Data(bytes: try XCTUnwrap(rep.bitmapData), count: rep.bytesPerRow * rep.pixelsHigh)
    }

    private func difference(_ a: Data, _ b: Data) throws -> Double {
        XCTAssertEqual(a.count, b.count)
        guard a.count == b.count, !a.isEmpty else { return 1 }
        return zip(a,b).reduce(0.0) { $0 + Double(abs(Int($1.0) - Int($1.1))) } / Double(a.count * 255)
    }
}
