import Foundation
import XCTest
@testable import PaperCore

final class PublicNativeSmokeTests: XCTestCase {
    func testReadSelfAuthoredPDFWithoutPython() throws {
        let stream = "BT /F1 14 Tf 1 0 0 1 40 150 Tm (Public fixture) Tj ET"
        let objects = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>", "<< /Length \(stream.utf8.count) >>\nstream\n\(stream)\nendstream", "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
        var pdf = "%PDF-1.7\n", offsets = [0]
        for (index, object) in objects.enumerated() { offsets.append(pdf.utf8.count); pdf += "\(index + 1) 0 obj\n\(object)\nendobj\n" }
        let xref = pdf.utf8.count
        pdf += "xref\n0 6\n0000000000 65535 f \n" + offsets.dropFirst().map { String(format: "%010d 00000 n \n", $0) }.joined()
        pdf += "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n\(xref)\n%%EOF\n"
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("PublicNative-\(UUID()).pdf")
        defer { try? FileManager.default.removeItem(at: url) }
        try Data(pdf.utf8).write(to: url)
        let engine = ContentEditEngine(python: URL(fileURLWithPath: "/unavailable/python"), script: URL(fileURLWithPath: "/unavailable/engine.py"))
        XCTAssertEqual(try engine.inspect(url, page: 0, cancelled: { false }).spans.map(\.text), ["Public fixture"])
    }
}
