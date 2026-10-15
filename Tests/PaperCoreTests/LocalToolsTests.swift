import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor
final class LocalToolsTests: XCTestCase {
    func testNumberingIntegratesAsSingleUndoableSourceProtectedTransaction() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let core = PDFWorkspace(); try core.createBlank()
        let token = try core.engineInput()
        let input = dir.appendingPathComponent("input.pdf"), output = dir.appendingPathComponent("output.pdf")
        try token.data.write(to: input)
        guard let qpdf = ExternalToolDiscovery.qpdf() else { throw XCTSkip("Optional qpdf not found; set PDFMASTER_QPDF or PATH") }
        XCTAssertTrue(FileManager.default.isExecutableFile(atPath: qpdf.path))
        var options = NumberOptions(); options.start = 5; options.prefix = "PAGE-"
        try OrganizeEngine(qpdf: qpdf, scratch: dir).number(input, output: output, options: options)
        try core.acceptEngineResult(Data(contentsOf: output), for: token)
        XCTAssertTrue(core.document?.string?.contains("PAGE-5") == true)
        XCTAssertTrue(core.isDirty)
        core.undo(); XCTAssertFalse(core.document?.string?.contains("PAGE-5") == true)
        XCTAssertFalse(core.isDirty)
        core.redo(); XCTAssertTrue(core.document?.string?.contains("PAGE-5") == true)
        XCTAssertEqual(try Data(contentsOf: input), token.data)
    }
}
