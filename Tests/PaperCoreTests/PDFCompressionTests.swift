import AppKit
import PDFKit
import XCTest
@testable import PaperCore

final class PDFCompressionTests: XCTestCase {
    private func requireEngine() throws -> URL {
        guard let engine = ExternalToolDiscovery.qpdf() else { throw XCTSkip("Optional qpdf not found; set PDFMASTER_QPDF or PATH") }
        return engine
    }

    func testImageCompressionPreservesTextLinksAnnotationsAndRotation() throws {
        let qpdf = try requireEngine()
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("source.pdf"), output = root.appendingPathComponent("compressed.pdf")
        let source = try fixture(); try source.write(to: input)
        let engine = OrganizeEngine(qpdf: qpdf, scratch: root)
        let result = try engine.compress(input, output: output, preset: .balanced)
        XCTAssertTrue(result.reduced)
        XCTAssertLessThan(result.after, result.before / 2, "Image compression must substantially reduce a large Flate RGB image")
        let document = try XCTUnwrap(PDFDocument(url: output)), page = try XCTUnwrap(document.page(at: 0))
        XCTAssertEqual(document.pageCount, 1)
        XCTAssertEqual(page.rotation, 90)
        XCTAssertTrue(document.string?.contains("SEARCHABLE COMPRESSION TEST") == true)
        XCTAssertEqual(page.annotations.count, 2)
        XCTAssertTrue(page.annotations.contains { $0.type == "Link" && $0.url?.absoluteString == "https://example.com/pdf" })
        XCTAssertTrue(page.annotations.contains { $0.type == "Highlight" && $0.contents == "Keep this comment" })
        XCTAssertEqual(try Data(contentsOf: input), source)
    }

    func testPresetsUseDistinctImageQualityWithoutChangingPageGeometry() throws {
        let qpdf = try requireEngine()
        XCTAssertEqual(PDFCompressionPreset.allCases.count, 4)
        XCTAssertEqual(PDFCompressionPreset.allCases.map(\.jpegQuality), [nil, 85, 65, 40])
        XCTAssertEqual(Set(PDFCompressionPreset.allCases.map(\.title)).count, 4)
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("source.pdf"), source = try fixture(); try source.write(to: input)
        let engine = OrganizeEngine(qpdf: qpdf, scratch: root)
        var sizes: [Int] = []
        for (index, preset) in PDFCompressionPreset.allCases.enumerated() {
            let output = root.appendingPathComponent("preset-\(index).pdf")
            let result = try engine.compress(input, output: output, preset: preset)
            XCTAssertLessThanOrEqual(result.after, result.before)
            XCTAssertEqual(result.before, source.count)
            XCTAssertEqual(result.after, try Data(contentsOf: output).count)
            let doc = try XCTUnwrap(PDFDocument(url: output)), page = try XCTUnwrap(doc.page(at: 0))
            XCTAssertEqual(page.bounds(for: .mediaBox), CGRect(x: 0, y: 0, width: 600, height: 400))
            XCTAssertEqual(page.rotation, 90)
            sizes.append(result.after)
        }
        XCTAssertGreaterThan(sizes[0], sizes[1])
        XCTAssertGreaterThan(sizes[1], sizes[2])
        XCTAssertGreaterThan(sizes[2], sizes[3])
        XCTAssertEqual(try Data(contentsOf: input), source)
    }

    func testNoSavingsCopiesOriginalBytesAndDoesNotOverwriteDestination() throws {
        let qpdf = try requireEngine()
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("source.pdf"), output = root.appendingPathComponent("unchanged.pdf")
        let source = pdf(objects: [Data("<< /Type /Catalog /Pages 2 0 R >>".utf8),
                                  Data("<< /Type /Pages /Kids [3 0 R] /Count 1 >>".utf8),
                                  Data("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> >>".utf8)])
        try source.write(to: input)
        let engine = OrganizeEngine(qpdf: qpdf, scratch: root)
        let result = try engine.compress(input, output: output)
        XCTAssertFalse(result.reduced)
        XCTAssertEqual(result.after, result.before)
        XCTAssertEqual(try Data(contentsOf: output), source)
        XCTAssertThrowsError(try engine.compress(input, output: output, preset: .smallSize)) {
            XCTAssertEqual($0 as? OrganizeError, .outputExists)
        }
        XCTAssertEqual(try Data(contentsOf: output), source)
    }

    func testCancellationCannotPublishAndCleansPrivateJob() throws {
        let qpdf = try requireEngine()
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("source.pdf"), output = root.appendingPathComponent("cancelled.pdf")
        let source = try fixture(); try source.write(to: input)
        var checks = 0
        XCTAssertThrowsError(try OrganizeEngine(qpdf: qpdf, scratch: root).compress(input, output: output, preset: .balanced, cancelled: {
            checks += 1; return checks >= 4
        })) { XCTAssertEqual($0 as? OrganizeError, .cancelled) }
        XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: root.path), ["source.pdf"])
        XCTAssertEqual(try Data(contentsOf: input), source)
    }

    func testSignedAndEncryptedDocumentsAreRejectedBeforeCompression() throws {
        let qpdf = try requireEngine()
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("signed.pdf"), output = root.appendingPathComponent("result.pdf")
        try fixture(signed: true).write(to: input)
        let engine = OrganizeEngine(qpdf: qpdf, scratch: root)
        XCTAssertThrowsError(try engine.validateCompressionInput(input)) {
            XCTAssertEqual($0 as? OrganizeError, .signed)
        }
        XCTAssertThrowsError(try engine.compress(input, output: output, preset: .balanced)) {
            XCTAssertEqual($0 as? OrganizeError, .signed)
        }
        let plain = root.appendingPathComponent("plain.pdf"), encrypted = root.appendingPathComponent("encrypted.pdf")
        try fixture().write(to: plain)
        let process = Process(); process.executableURL = qpdf
        process.arguments = ["--encrypt", "test-password", "test-owner-password", "256", "--", plain.path, encrypted.path]
        try process.run(); process.waitUntilExit(); XCTAssertEqual(process.terminationStatus, 0)
        XCTAssertThrowsError(try engine.compress(encrypted, output: output, preset: .balanced)) {
            XCTAssertEqual($0 as? OrganizeError, .encrypted)
        }
        XCTAssertThrowsError(try engine.validateCompressionInput(encrypted)) {
            XCTAssertEqual($0 as? OrganizeError, .encrypted)
        }
        XCTAssertNoThrow(try engine.validateCompressionInput(plain))
        XCTAssertThrowsError(try engine.validateCompressionInput(plain, cancelled: { true })) {
            XCTAssertEqual($0 as? OrganizeError, .cancelled)
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
    }

    private func directory() throws -> URL {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("PDFCompressionTests-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        return root
    }

    private func fixture(signed: Bool = false) throws -> Data {
        let width = 1024, height = 768
        var pixels = Data(count: width * height * 3)
        pixels.withUnsafeMutableBytes { raw in
            let bytes = raw.bindMemory(to: UInt8.self)
            var seed: UInt32 = 7
            for y in 0..<height { for x in 0..<width {
                seed = 1664525 &* seed &+ 1013904223
                let noise = Int((seed >> 24) & 31)
                let i = (y * width + x) * 3
                bytes[i] = UInt8((x / 5 + noise) % 256)
                bytes[i + 1] = UInt8((y / 4 + noise) % 256)
                bytes[i + 2] = UInt8(((x + y) / 8 + noise) % 256)
            } }
        }
        let deflate = try (pixels as NSData).compressed(using: .zlib) as Data
        var a: UInt32 = 1, b: UInt32 = 0
        for byte in pixels { a = (a + UInt32(byte)) % 65521; b = (b + a) % 65521 }
        let checksum = (b << 16) | a
        var image = Data([0x78, 0x9c]); image.append(deflate)
        image.append(contentsOf: [UInt8((checksum >> 24) & 255), UInt8((checksum >> 16) & 255), UInt8((checksum >> 8) & 255), UInt8(checksum & 255)])
        func stream(_ header: String, _ data: Data) -> Data {
            var value = Data("<< \(header) /Length \(data.count) >>\nstream\n".utf8)
            value.append(data); value.append(Data("\nendstream".utf8)); return value
        }
        var objects: [Data] = [
            Data("<< /Type /Catalog /Pages 2 0 R >>".utf8),
            Data("<< /Type /Pages /Kids [3 0 R] /Count 1 >>".utf8),
            Data("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 400] /Rotate 90 /Resources << /XObject << /Im0 5 0 R >> /Font << /F1 6 0 R >> >> /Contents 4 0 R /Annots [7 0 R 8 0 R] >>".utf8),
            stream("", Data("q 540 0 0 280 30 20 cm /Im0 Do Q\nBT /F1 18 Tf 30 350 Td (SEARCHABLE COMPRESSION TEST) Tj ET".utf8)),
            stream("/Type /XObject /Subtype /Image /Width \(width) /Height \(height) /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode", image),
            Data("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".utf8),
            Data("<< /Type /Annot /Subtype /Link /Rect [30 310 200 330] /A << /S /URI /URI (https://example.com/pdf) >> >>".utf8),
            Data("<< /Type /Annot /Subtype /Highlight /Rect [30 345 250 365] /QuadPoints [30 365 250 365 30 345 250 345] /C [1 1 0] /Contents (Keep this comment) >>".utf8)
        ]
        if signed { objects.append(Data("<< /FT /Sig /Type /Sig /ByteRange [0 1 2 3] >>".utf8)) }
        return pdf(objects: objects)
    }

    private func pdf(objects: [Data]) -> Data {
        var pdf = Data("%PDF-1.7\n".utf8), offsets = [0]
        for (index, object) in objects.enumerated() {
            offsets.append(pdf.count); pdf.append(Data("\(index + 1) 0 obj\n".utf8)); pdf.append(object); pdf.append(Data("\nendobj\n".utf8))
        }
        let xref = pdf.count
        pdf.append(Data("xref\n0 \(objects.count + 1)\n0000000000 65535 f \n".utf8))
        for offset in offsets.dropFirst() { pdf.append(Data(String(format: "%010d 00000 n \n", offset).utf8)) }
        pdf.append(Data("trailer\n<< /Size \(objects.count + 1) /Root 1 0 R >>\nstartxref\n\(xref)\n%%EOF\n".utf8))
        return pdf
    }
}
