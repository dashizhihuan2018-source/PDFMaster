import Foundation
import PDFKit
import XCTest
@testable import PaperCore

final class PDFWatermarkTests: XCTestCase {
    private var engine: PDFWatermarkEngine { PDFWatermarkEngine() }
    private func fixture(marked: Bool = true, malformed: Bool = false, signed: Bool = false, annotations: Bool = true, customStream: String? = nil) -> Data {
        let body = "BT /F1 14 Tf 40 150 Td (Body preserved) Tj ET\n"
        let mark = marked ? "/Artifact << /Type /Pagination /Subtype /Watermark >> BDC q BT /F1 20 Tf 40 70 Td (Watermark text) Tj ET \(malformed ? "" : "Q EMC")\n" : ""
        let stream = customStream ?? (body + mark)
        let annots = annotations ? "/Annots [7 0 R 8 0 R]" : ""
        let objects = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 2 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R \(annots) >>", "<< /Length \(stream.utf8.count) >>\nstream\n\(stream)endstream", "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R \(annots) >>", "<< /Type /Annot /Subtype /Watermark /Rect [20 20 100 50] /Contents (Explicit watermark) >>", "<< /Type /Annot /Subtype /Text /Rect [150 20 170 40] /Contents (Ordinary note) \(signed ? "/ByteRange [0 1 2 3]" : "") >>"]
        var text = "%PDF-1.7\n", offsets = [0]
        for (index, object) in objects.enumerated() { offsets.append(text.utf8.count); text += "\(index + 1) 0 obj\n\(object)\nendobj\n" }
        let start = text.utf8.count
        text += "xref\n0 \(objects.count + 1)\n0000000000 65535 f \n" + offsets.dropFirst().map { String(format: "%010d 00000 n \n", $0) }.joined()
        text += "trailer\n<< /Size \(objects.count + 1) /Root 1 0 R >>\nstartxref\n\(start)\n%%EOF\n"
        return Data(text.utf8)
    }
    private func withFiles(_ body: (URL, URL) throws -> Void) throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("WatermarkTests-\(UUID())")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: dir) }
        let input = dir.appendingPathComponent("source.pdf"); try fixture().write(to: input)
        try body(input, dir.appendingPathComponent("copy.pdf"))
    }
    func testRangeScopedCandidatesRemoveOnlyExplicitObjectsAndCloneSharedStream() throws {
        try withFiles { source, output in
            let original = try Data(contentsOf: source)
            let scan = try engine.scan(source, pages: [0], cancelled: { false })
            XCTAssertEqual(scan.candidates.count, 2)
            XCTAssertEqual(Set(scan.candidates.map(\.kind)), [.annotation, .markedContent])
            XCTAssertEqual(scan, try engine.scan(source, pages: [0], cancelled: { false }))
            try engine.remove(source, scan: scan, selectedIDs: Set(scan.candidates.map(\.id)), output: output, cancelled: { false })
            let reopened = try XCTUnwrap(PDFDocument(url: output))
            XCTAssertTrue(reopened.page(at: 0)?.string?.contains("Body preserved") == true)
            XCTAssertFalse(reopened.page(at: 0)?.string?.contains("Watermark text") == true)
            XCTAssertTrue(reopened.page(at: 1)?.string?.contains("Watermark text") == true)
            XCTAssertEqual(reopened.page(at: 0)?.annotations.map(\.contents), ["Ordinary note"])
            XCTAssertEqual(reopened.page(at: 1)?.annotations.count, 2)
            XCTAssertEqual(try Data(contentsOf: source), original)
        }
    }
    func testOnlySelectedCandidateIsRemovedAndNormalAnnotationRemains() throws {
        try withFiles { source, output in
            let scan = try engine.scan(source, pages: [0, 1], cancelled: { false })
            let candidate = try XCTUnwrap(scan.candidates.first { $0.kind == .annotation && $0.page == 1 })
            try engine.remove(source, scan: scan, selectedIDs: [candidate.id], output: output, cancelled: { false })
            XCTAssertEqual(PDFDocument(url: output)?.page(at: 0)?.annotations.count, 2)
            XCTAssertEqual(PDFDocument(url: output)?.page(at: 1)?.annotations.map(\.contents), ["Ordinary note"])
            XCTAssertTrue(PDFDocument(url: output)?.page(at: 1)?.string?.contains("Watermark text") == true)
        }
    }
    func testEmptyStaleUnknownAndCancelledRequestsNeverPublishOutput() throws {
        try withFiles { source, output in
            let scan = try engine.scan(source, pages: [0], cancelled: { false })
            XCTAssertThrowsError(try engine.remove(source, scan: scan, selectedIDs: [], output: output, cancelled: { false }))
            XCTAssertThrowsError(try engine.remove(source, scan: scan, selectedIDs: ["invented"], output: output, cancelled: { false }))
            XCTAssertThrowsError(try engine.remove(source, scan: scan, selectedIDs: Set(scan.candidates.map(\.id)), output: output, cancelled: { true }))
            try fixture(marked: false).write(to: source)
            XCTAssertThrowsError(try engine.remove(source, scan: scan, selectedIDs: Set(scan.candidates.map(\.id)), output: output, cancelled: { false }))
            XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
        }
    }
    func testMalformedContentIsNotOfferedAndSignaturesAreRejected() throws {
        try withFiles { source, _ in
            try fixture(malformed: true).write(to: source)
            let scan = try engine.scan(source, pages: [0], cancelled: { false })
            XCTAssertFalse(scan.candidates.contains { $0.kind == .markedContent })
            XCTAssertFalse(scan.limitations.isEmpty)
            try fixture(signed: true).write(to: source)
            XCTAssertThrowsError(try engine.scan(source, pages: [0], cancelled: { false }))
        }
    }
    func testSeparateOutputRejectsSourceSymlinkAndHardlinkAliases() throws {
        try withFiles { source, output in
            let original = try Data(contentsOf: source)
            let scan = try engine.scan(source, pages: [0], cancelled: { false })
            for destination in [source, output] {
                if destination == output { try FileManager.default.createSymbolicLink(at: output, withDestinationURL: source) }
                XCTAssertThrowsError(try engine.remove(source, scan: scan, selectedIDs: Set(scan.candidates.map(\.id)), output: destination, cancelled: { false }))
            }
            try FileManager.default.removeItem(at: output); try FileManager.default.linkItem(at: source, to: output)
            XCTAssertThrowsError(try engine.remove(source, scan: scan, selectedIDs: Set(scan.candidates.map(\.id)), output: output, cancelled: { false }))
            XCTAssertEqual(try Data(contentsOf: source), original)
        }
    }

    func testUnmarkedBodyIsNotMistakenForWatermarkAndEmptyScanCannotExport() throws {
        try withFiles { source, output in
            try fixture(marked: false, annotations: false).write(to: source)
            let scan = try engine.scan(source, pages: [0, 1], cancelled: { false })
            XCTAssertTrue(scan.candidates.isEmpty)
            XCTAssertFalse(scan.limitations.isEmpty)
            XCTAssertThrowsError(try engine.remove(source, scan: scan, selectedIDs: [], output: output, cancelled: { false }))
            XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
        }
    }

    func testMarkedSpanWithLeakingCurrentPathIsNeverOffered() throws {
        try withFiles { source, _ in
            let stream = "0 0 m /Artifact << /Subtype /Watermark >> BDC q 100 100 l S Q EMC 200 200 l S\n"
            try fixture(annotations: false, customStream: stream).write(to: source)
            XCTAssertTrue(try engine.scan(source, pages: [0], cancelled: { false }).candidates.isEmpty)
        }
    }

    func testUnknownCompatibilityOperatorsAreRefusedRatherThanAssumedSafe() throws {
        try withFiles { source, _ in
            let stream = "/Artifact << /Subtype /Watermark >> BDC q BX PrivateDrawOperator EX Q EMC\n"
            try fixture(annotations: false, customStream: stream).write(to: source)
            XCTAssertTrue(try engine.scan(source, pages: [0], cancelled: { false }).candidates.isEmpty)
        }
    }
}
