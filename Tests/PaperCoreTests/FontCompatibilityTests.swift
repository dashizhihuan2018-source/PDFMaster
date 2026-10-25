import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor
final class FontCompatibilityTests: XCTestCase {
    private lazy var publicFixture = try! PublicContentFixture()
    func testNativeRepairPreservesCFFOutlinesAndCorrectsItsPDFType() throws {
        let source = NativeFontFixture.pdf()
        XCTAssertTrue(PDFFontCompatibility.needsRepair(source))
        XCTAssertEqual(try NativeFontCFF.charset(NativeFontFixture.program).cids, [0, 10, 200])
        XCTAssertNoThrow(try NativeFontRepair.repair(source))
        let result = PDFFontCompatibility.normalize(source)
        XCTAssertNotNil(result.notice)
        let pdf = try XCTUnwrap(CGPDFDocument(CGDataProvider(data: result.data as CFData)!))
        let page = try XCTUnwrap(pdf.page(at: 1))
        var resources: CGPDFDictionaryRef?, fonts: CGPDFDictionaryRef?, font: CGPDFDictionaryRef?
        var descendants: CGPDFArrayRef?, child: CGPDFDictionaryRef?, descriptor: CGPDFDictionaryRef?, program: CGPDFStreamRef?
        XCTAssertTrue(CGPDFDictionaryGetDictionary(page.dictionary!, "Resources", &resources))
        XCTAssertTrue(CGPDFDictionaryGetDictionary(resources!, "Font", &fonts))
        XCTAssertTrue(CGPDFDictionaryGetDictionary(fonts!, "F1", &font))
        XCTAssertTrue(CGPDFDictionaryGetArray(font!, "DescendantFonts", &descendants))
        XCTAssertTrue(CGPDFArrayGetDictionary(descendants!, 0, &child))
        var subtype: UnsafePointer<CChar>?
        XCTAssertTrue(CGPDFDictionaryGetName(child!, "Subtype", &subtype))
        XCTAssertEqual(subtype.map(String.init(cString:)), "CIDFontType0", "Repair must retain CFF, not depend on a Python TrueType converter")
        XCTAssertTrue(CGPDFDictionaryGetDictionary(child!, "FontDescriptor", &descriptor))
        XCTAssertTrue(CGPDFDictionaryGetStream(descriptor!, "FontFile3", &program))
        guard let program else { return XCTFail("A native CFF program must remain embedded as FontFile3") }
        var format = CGPDFDataFormat.raw
        XCTAssertEqual(CGPDFStreamCopyData(program, &format) as Data?, NativeFontFixture.denseProgram,
                       "Only charset CID labels may change; every outline byte remains identical")
        XCTAssertFalse(PDFFontCompatibility.needsRepair(result.data))
        XCTAssertEqual(try XCTUnwrap(PDFDocument(data: result.data)).page(at: 0)?.string, "AB")
        let reference = try XCTUnwrap(PDFDocument(data: NativeFontFixture.pdf(correct: true)))
        let repaired = try XCTUnwrap(PDFDocument(data: result.data))
        XCTAssertGreaterThan(try difference(pixels(try XCTUnwrap(PDFDocument(data: source)), page: 0), pixels(reference, page: 0)), 0.001)
        XCTAssertLessThan(try difference(pixels(repaired, page: 0), pixels(reference, page: 0)), 0.001)
    }

    func testNativeRepairLeavesAValidCFFByteForByteUnchanged() {
        let source = NativeFontFixture.pdf(correct: true)
        let result = PDFFontCompatibility.normalize(source)
        XCTAssertEqual(source, result.data)
        XCTAssertNil(result.notice)
    }

    func testNativeRepairRejectsWrongCIDMapWithoutChangingInput() {
        let source = NativeFontFixture.pdf(mapping: Data([0, 0, 0, 11, 0, 200]))
        let result = PDFFontCompatibility.normalize(source)
        XCTAssertEqual(source, result.data)
        XCTAssertNotNil(result.notice)
    }

    func testNativeRepairRejectsSignatureBearingPDFWithoutChangingInput() {
        let source = NativeFontFixture.pdf(signed: true)
        let result = PDFFontCompatibility.normalize(source)
        XCTAssertEqual(source, result.data)
        XCTAssertNotNil(result.notice)
    }

    func testNativeRepairRejectsUnsupportedVerticalEncoding() {
        let source = NativeFontFixture.pdf(vertical: true)
        let result = PDFFontCompatibility.normalize(source)
        XCTAssertEqual(source, result.data)
        XCTAssertNotNil(result.notice)
    }

    func testNativeRepairRejectsNonCFFGarbageWithoutChangingInput() {
        let source = NativeFontFixture.pdf(fontProgram: Data([1, 0, 4, 1, 255, 255, 255, 255]))
        let result = PDFFontCompatibility.normalize(source)
        XCTAssertEqual(source, result.data)
        XCTAssertNotNil(result.notice)
    }

    func testNativeCFFReaderRejectsEveryTruncatedDirectory() {
        for count in 0..<NativeFontFixture.program.count {
            XCTAssertThrowsError(try NativeFontCFF.charset(NativeFontFixture.program.prefix(count)), "prefix \(count)")
        }
    }

    func testNativeCFFReaderDensifiesBothRangeCharsetFormatsWithoutMovingOutlines() throws {
        for (original, expected) in [([UInt8(1), 0, 10, 1, 0], [UInt8(1), 0, 1, 1, 0]),
                                     ([UInt8(2), 0, 10, 0, 1], [UInt8(2), 0, 1, 0, 1])] {
            var program = NativeFontFixture.program
            let range = try XCTUnwrap(program.range(of: Data([0, 0, 10, 0, 200])))
            program.replaceSubrange(range, with: original)
            let parsed = try NativeFontCFF.charset(program)
            XCTAssertEqual(parsed.cids, [0, 10, 11])
            program.replaceSubrange(range, with: expected)
            XCTAssertEqual(parsed.denseProgram, program)
        }
    }

    func testNativeRepairRejectsEncryptedPDFWithoutChangingInput() throws {
        let qpdf = URL(fileURLWithPath: ProcessInfo.processInfo.environment["PDFMASTER_TEST_QPDF"] ?? "/unavailable/qpdf")
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("NativeFontEncryption-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: dir) }
        let input = dir.appendingPathComponent("source.pdf"), output = dir.appendingPathComponent("encrypted.pdf")
        try NativeFontFixture.pdf().write(to: input)
        let task = Process(); task.executableURL = qpdf
        task.arguments = [input.path, "--encrypt", "synthetic-user", "synthetic-owner", "256", "--", output.path]
        task.standardOutput = FileHandle.nullDevice; task.standardError = FileHandle.nullDevice
        try task.run(); task.waitUntilExit()
        XCTAssertEqual(task.terminationStatus, 0)
        let encrypted = try Data(contentsOf: output)
        XCTAssertThrowsError(try NativeFontRepair.repair(encrypted))
        XCTAssertEqual(PDFFontCompatibility.normalize(encrypted).data, encrypted)
        XCTAssertEqual(try Data(contentsOf: output), encrypted)
    }

    func testSelfAuthoredNormalFontsRemainByteAndPixelIdentical() throws {
        let sample = try PublicContentFixture(pageCount: 5, title: "示例古建", imagePageCount: 4)
        let url = sample.source
        let original = try Data(contentsOf: url), before = try XCTUnwrap(PDFDocument(data: original))
        XCTAssertEqual(before.pageCount, 5)
        XCTAssertFalse(PDFFontCompatibility.needsRepair(original))
        let normalized = PDFFontCompatibility.normalize(original)
        XCTAssertNil(normalized.notice)
        XCTAssertEqual(normalized.data, original)
        let after = try XCTUnwrap(PDFDocument(data: normalized.data))
        for page in 0..<5 {
            XCTAssertEqual(before.page(at: page)?.string, after.page(at: page)?.string)
            XCTAssertEqual(try pixels(before, page: page), try pixels(after, page: page))
        }
        XCTAssertEqual(try Data(contentsOf: url), original)
    }

    func testMalformedEmbeddedCFFRendersLikeCorrectTrueTypeAndSurvivesSave() throws {
        let env = ProcessInfo.processInfo.environment
        let generated = FileManager.default.temporaryDirectory.appendingPathComponent("FontFixture-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: generated) }
        var source = env["PDFMASTER_FONT_FIXTURE"], reference = env["PDFMASTER_FONT_REFERENCE"]
        if source == nil || reference == nil {
            try FileManager.default.createDirectory(at: generated, withIntermediateDirectories: false)
            try NativeFontFixture.pdf().write(to: generated.appendingPathComponent("broken.pdf"))
            try NativeFontFixture.pdf(trueType: true).write(to: generated.appendingPathComponent("reference.pdf"))
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

/// Our own three synthetic outlines; not a system/commercial font or user PDF.
/// The compact bytes encode self-authored outlines; the fixture needs no
/// interpreter, external font tool, or third-party PDF manipulation library.
private enum NativeFontFixture {
    static let program = Data(hex: "0100040100010101135044464d617374657253796e746865746963000101011ff81bf81c8b0c1e8b8bf950f95005f75d0c22f10ff60c25f72c0c24f7031100030101060e2041646f62654964656e746974795044464d617374657253796e746865746963000000000a00c80000000000030101041023fa7c0efa7c8b16f950f950fd50060efa7c8b16f950f95006fbf2fbf2fbf2f7f2050e0001010109f81d0c268bf73912")
    static var denseProgram: Data {
        var result = program
        let range = result.range(of: Data([0, 0, 10, 0, 200, 0, 0, 0]))!
        result.replaceSubrange(range, with: Data([0, 0, 1, 0, 2, 0, 0, 0]))
        return result
    }
    // Independent TrueType rendering oracle for the same self-authored outlines.
    static let trueTypeProgram = Data(hex: """
    00010000000a0080000300204f532f324138436e0000012800000060636d
    6170000c00950000019000000034676c796625e32c37000001cc00000038
    686561642fda1e38000000ac0000003668686561062e049e000000e40000
    0024686d747803e8000000000188000000086c6f6361000c001c000001c4
    000000086d6178700005000700000108000000206e616d6588458bf80000
    020400000099706f7374fe032e33000002a00000003a0001000000010000
    56b6013a5f0f3cf5000303e800000000e703ed4e00000000e703ed4e0000
    000002bc02bc000000030002000000000000000100000370ff88000003e8
    0000012c02bc000100000000000000000000000000000001000100000003
    0005000100000000000200000000000000000000000000000000000303e8
    019000050004000000000000000000000000000000000000000000000000
    0000000000000000000000010000000000000000000000003f3f3f3f0000
    004100420000000000000000000000000000000000000000000000000020
    000003e80000000000000000000200000003000000140003000100000014
    00040020000000040004000100000042ffff00000041ffffffc000010000
    000000000000000c001c00010000000002bc02bc000300003121112102bc
    fd4402bc00010000000002bc02bc00040000312111010102bcfea2fea202
    bcfea2015e0000000006004e000100000000000100120000000100000000
    000200070012000100000000000600120000000300010409000100240019
    0003000104090002000e003d0003000104090006002400195044464d6173
    74657253796e746865746963526567756c6172005000440046004d006100
    7300740065007200530079006e0074006800650074006900630052006500
    670075006c00610072000000000200000000000000000000000000000000
    000000000000000000000000000000030000010201030863696430303031
    300863696430303230300000
    """)

    static func pdf(correct: Bool = false, mapping: Data = Data([0, 0, 0, 10, 0, 200]), signed: Bool = false,
                    vertical: Bool = false, fontProgram: Data = program, trueType: Bool = false) -> Data {
        func ascii(_ text: String) -> Data { Data(text.utf8) }
        func stream(_ data: Data, extra: String = "") -> Data {
            ascii("<< /Length \(data.count) \(extra) >>\nstream\n") + data + ascii("\nendstream")
        }
        let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def /CMapName /PDFMasterSyntheticEncoding def /CMapType 1 def /WMode 0 def 1 begincodespacerange <0000> <ffff> endcodespacerange 3 begincidchar <0000> 0 <0001> 10 <0002> 200 endcidchar endcmap CMapName currentdict /CMap defineresource pop end end"
        let unicode = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /TestUCS def /CMapType 2 def 1 begincodespacerange <0000> <ffff> endcodespacerange 2 beginbfchar <0001> <0041> <0002> <0042> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end"
        let fontFile = correct && !trueType ? "/FontFile3 5 0 R" : "/FontFile2 5 0 R"
        let childType = correct && !trueType ? "/CIDFontType0" : "/CIDFontType2"
        let map = trueType ? "/CIDToGIDMap /Identity" : (correct ? "" : "/CIDToGIDMap 7 0 R")
        let encoding = vertical ? "/Identity-V" : "/Identity-H"
        let signature = signed ? "/AcroForm << /Fields [13 0 R] /SigFlags 3 >>" : ""
        let objects: [Data] = [
            ascii("<< /Type /Catalog /Pages 2 0 R \(signature) >>"),
            ascii("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            ascii("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 10 0 R >> >> /Contents 11 0 R >>"),
            ascii("<< >>"),
            stream(trueType ? trueTypeProgram : (correct ? denseProgram : fontProgram), extra: correct && !trueType ? "/Subtype /CIDFontType0C" : ""),
            ascii("<< /Type /FontDescriptor /FontName /PDFMasterSynthetic /Flags 32 /FontBBox [0 0 700 700] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 \(fontFile) >>"),
            stream(mapping),
            ascii("<< /Type /Font /Subtype \(childType) /BaseFont /PDFMasterSynthetic /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 1000 /FontDescriptor 6 0 R \(map) >>"),
            stream(ascii(unicode)),
            ascii("<< /Type /Font /Subtype /Type0 /BaseFont /PDFMasterSynthetic /Encoding \(encoding) /DescendantFonts [8 0 R] /ToUnicode 9 0 R >>"),
            stream(ascii("BT /F1 70 Tf 30 70 Td <00010002> Tj ET")),
            stream(ascii(cmap)),
            ascii(signed ? "<< /Type /Annot /Subtype /Widget /FT /Sig /T (Synthetic signature) /V 14 0 R /Rect [0 0 1 1] >>" : "<< >>"),
            ascii(signed ? "<< /Type /Sig /ByteRange [0 0 0 0] /Contents <> >>" : "<< >>")
        ]
        var data = ascii("%PDF-1.7\n"), offsets = [0]
        for (index, object) in objects.enumerated() {
            offsets.append(data.count)
            data.append(ascii("\(index + 1) 0 obj\n")); data.append(object); data.append(ascii("\nendobj\n"))
        }
        let start = data.count
        data.append(ascii("xref\n0 \(objects.count + 1)\n0000000000 65535 f \n"))
        for offset in offsets.dropFirst() { data.append(ascii(String(format: "%010d 00000 n \n", offset))) }
        data.append(ascii("trailer\n<< /Size \(objects.count + 1) /Root 1 0 R >>\nstartxref\n\(start)\n%%EOF\n"))
        return data
    }
}

private extension Data {
    init(hex: String) {
        let values = Array(hex.filter { !$0.isWhitespace }.utf8)
        self.init(stride(from: 0, to: values.count, by: 2).map { index in
            UInt8(String(bytes: values[index...index+1], encoding: .utf8)!, radix: 16)!
        })
    }
}
