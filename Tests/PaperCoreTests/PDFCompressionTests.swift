import AppKit
import PDFKit
import ImageIO
import UniformTypeIdentifiers
import XCTest
@testable import PaperCore

final class PDFCompressionTests: XCTestCase {
    private var qpdf: URL {
        get throws {
            guard let engine = ExternalToolDiscovery.qpdf() else { throw XCTSkip("Optional qpdf not found; set PDFMASTER_QPDF or PATH") }
            return engine
        }
    }

    func testExistingJPEGCanBeRecompressedAtLowerQuality() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("jpeg.pdf"), output = root.appendingPathComponent("compressed.pdf")
        let source = try fixture(jpeg: true); try source.write(to: input)
        let result = try OrganizeEngine(qpdf: qpdf, scratch: root).compress(input, output: output, preset: .smallSize)
        XCTAssertLessThan(result.after, result.before / 2, "The bundled qpdf must recompress existing high-quality JPEG, not merely Flate images")
        XCTAssertEqual(try Data(contentsOf: input), source)
    }

    func testSmallPresetDownsamplesOversizedJPEGWithoutRasterizingPage() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("large-jpeg.pdf"), output = root.appendingPathComponent("compressed.pdf")
        let source = try fixture(jpeg: true, width: 3000, height: 2000); try source.write(to: input)
        let result = try OrganizeEngine(qpdf: qpdf, scratch: root).compress(input, output: output, preset: .smallSize)
        XCTAssertTrue(result.reduced)
        let document = try XCTUnwrap(PDFDocument(url: output)), page = try XCTUnwrap(document.page(at: 0))
        XCTAssertEqual(document.pageCount, 1)
        XCTAssertEqual(page.rotation, 90)
        XCTAssertEqual(page.bounds(for: .mediaBox), CGRect(x: 0, y: 0, width: 600, height: 400))
        XCTAssertTrue(document.string?.contains("SEARCHABLE COMPRESSION TEST") == true)
        XCTAssertTrue(page.annotations.contains { $0.type == "Link" && $0.url?.absoluteString == "https://example.com/pdf" })
        XCTAssertTrue(page.annotations.contains { $0.type == "Highlight" && $0.contents == "Keep this comment" })
        let dimensions = try imageDimensions(output)
        XCTAssertLessThan(dimensions.width, 3000, "Small-file compression must downsample oversized source images")
        XCTAssertLessThan(dimensions.height, 2000)
        XCTAssertEqual(try Data(contentsOf: input), source)
    }

    func testDPIUsesActualImageDrawingSizeForEveryPreset() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("dpi.pdf")
        try fixture(jpeg: true, width: 1200, height: 800, drawing: "q 144 0 0 96 20 20 cm /Im0 Do Q").write(to: input)
        let engine = try OrganizeEngine(qpdf: qpdf, scratch: root)
        for (preset, expected) in [(PDFCompressionPreset.highQuality, (600, 400)), (.balanced, (300, 200)), (.smallSize, (144, 96))] {
            let output = root.appendingPathComponent("\(preset).pdf")
            _ = try engine.compress(input, output: output, preset: preset)
            let dimensions = try imageDimensions(output)
            XCTAssertEqual(dimensions.width, expected.0)
            XCTAssertEqual(dimensions.height, expected.1)
        }
        XCTAssertEqual(PDFCompressionPreset.allCases.map(\.imageDPI), [nil, 300, 150, 72])
    }

    func testNormalPagesCanExceedOneHundredThousandOperationsInTotal() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("many-pages.pdf")
        // 12 * 9,000 ordinary path operators: normal pages, >100k document work.
        try fixture(jpeg: true, width: 1200, height: 800,
            drawing: String(repeating: "n\n", count: 9_000) + "q 144 0 0 96 0 0 cm /Im0 Do Q",
            repeatedPages: 12).write(to: input)
        for (preset, expectedWidth) in [(PDFCompressionPreset.highQuality, 600), (.balanced, 300), (.smallSize, 144)] {
            let output = root.appendingPathComponent("\(preset).pdf")
            _ = try OrganizeEngine(qpdf: qpdf, scratch: root).compress(input, output: output, preset: preset)
            XCTAssertEqual(try imageDimensions(output).width, expectedWidth)
            XCTAssertEqual(PDFDocument(url: output)?.pageCount, 12)
        }
    }

    func testSinglePageAndWholeDocumentOperationLimitsStillSkipBeforeImageExtraction() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        for (pages, operations) in [(1, 100_001), (501, 10_000)] {
            let input = root.appendingPathComponent("limit-\(pages).pdf")
            try fixture(jpeg: true, drawing: String(repeating: "n\n", count: operations) + "q 72 0 0 72 0 0 cm /Im0 Do Q",
                        repeatedPages: pages).write(to: input)
            var extractions = 0
            let result = try PDFImageCompression.prepare(input: input, inspection: inspect(input), job: root,
                dpi: 72, quality: 40, cancelled: { false }, run: { _, _, _ in extractions += 1; return Data() })
            XCTAssertNil(result)
            XCTAssertEqual(extractions, 0)
        }
    }

    func testNestedFormsShareTheirRootPageOperationBudget() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("repeated-form.pdf")
        // Two invocations individually fit 100k; together the root page does not.
        try fixture(jpeg: true, nestedSecondPage: true, secondPageRepeats: 2, innerFormOperations: 50_001).write(to: input)
        var extractions = 0
        let update = try PDFImageCompression.prepare(input: input, inspection: inspect(input), job: root,
            dpi: 72, quality: 40, cancelled: { false }, run: { _, _, _ in extractions += 1; return Data() })
        XCTAssertNil(update)
        XCTAssertEqual(extractions, 0)
    }

    func testSequentialImageWorkCanExceedOneHundredMegapixelsWithoutDecodingOverlargeImages() throws {
        XCTAssertEqual(try admittedImageExtractions(copies: 5), 5,
            "A cumulative processing budget is not the peak single-image allocation limit")
    }

    func testOneGigapixelCumulativeImageWorkLimitStillSkipsFurtherImages() throws {
        XCTAssertEqual(try admittedImageExtractions(copies: 41), 40)
    }

    private func admittedImageExtractions(copies: Int) throws -> Int {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("image-work.pdf")
        try fixture(jpeg: true, imageCopies: copies).write(to: input)
        var inspection = try inspect(input)
        var entries = try XCTUnwrap(inspection["qpdf"] as? [Any])
        var objects = try XCTUnwrap(entries[1] as? [String: Any])
        for key in Array(objects.keys) {
            guard var object = objects[key] as? [String: Any], var stream = object["stream"] as? [String: Any],
                  var dictionary = stream["dict"] as? [String: Any], dictionary["/Subtype"] as? String == "/Image" else { continue }
            dictionary["/Width"] = 5_000; dictionary["/Height"] = 5_000 // 25MP each; no pixels allocated by this admission test.
            stream["dict"] = dictionary; object["stream"] = stream; objects[key] = object
        }
        entries[1] = objects; inspection["qpdf"] = entries
        var extractions = 0
        _ = try PDFImageCompression.prepare(input: input, inspection: inspection, job: root, dpi: 72, quality: 40,
            cancelled: { false }, run: { _, output, _ in
                extractions += 1
                if let output { XCTAssertTrue(FileManager.default.createFile(atPath: output.path, contents: Data())) }
                return Data()
            })
        return extractions
    }

    func testSharedImageUsesLargestDrawingAcrossRotatedPagesAndNestedForms() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("shared.pdf"), output = root.appendingPathComponent("result.pdf")
        // First occurrence is tiny. Page two paints through two Forms with Matrix transforms:
        // Page scale(108,72) * rotation(90) * Form scale(2,3): axis lengths 144,324 points.
        let source = try fixture(jpeg: true, width: 1200, height: 800,
            drawing: "q 20 0 0 10 0 0 cm /Im0 Do Q", nestedSecondPage: true)
        try source.write(to: input)
        _ = try OrganizeEngine(qpdf: qpdf, scratch: root).compress(input, output: output, preset: .balanced)
        let dimensions = try imageDimensions(output)
        // Preserve aspect ratio: largest per-axis requested ratio wins (675/800).
        XCTAssertEqual(dimensions.width, 1013)
        XCTAssertEqual(dimensions.height, 675)
        let doc = try XCTUnwrap(PDFDocument(url: output))
        XCTAssertEqual(doc.pageCount, 2)
        XCTAssertEqual(doc.page(at: 1)?.rotation, 270)
        XCTAssertTrue(doc.string?.contains("SEARCHABLE COMPRESSION TEST") == true)
    }

    func testMaskAndUnreliableGraphicsStateKeepOriginalImageDimensions() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        for (index, drawing, extras) in [(0, "q 72 0 0 72 0 0 cm /Im0 Do Q", "/Mask [0 0 0 0 0 0]"),
                                         (1, "Q q 72 0 0 72 0 0 cm /Im0 Do Q", "")] {
            let input = root.appendingPathComponent("unsafe-\(index).pdf"), output = root.appendingPathComponent("result-\(index).pdf")
            try fixture(jpeg: true, drawing: drawing, extraImageEntries: extras).write(to: input)
            _ = try OrganizeEngine(qpdf: qpdf, scratch: root).compress(input, output: output, preset: .smallSize)
            let dimensions = try imageDimensions(output)
            XCTAssertEqual(dimensions.width, 1024)
            XCTAssertEqual(dimensions.height, 768)
        }
    }

    func testDefaultColorSpaceUsageKeepsSharedImageBytesIncludingNestedForms() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let rgb = "/ColorSpace << /DefaultRGB [/CalRGB << /WhitePoint [0.9505 1 1.089] /Gamma [1.8 1.8 1.8] >>] >>"
        let gray = "/ColorSpace << /DefaultGray [/CalGray << /WhitePoint [0.9505 1 1.089] /Gamma 1.8 >>] >>"
        for (index, isGray, pageColors, formColors, nested) in [(0, false, rgb, "", false), (1, true, gray, "", false),
                                                               (2, false, "", rgb, true), (3, false, rgb, "", true)] {
            let input = root.appendingPathComponent("default-\(index).pdf"), output = root.appendingPathComponent("result-\(index).pdf")
            try fixture(jpeg: true, drawing: "q 72 0 0 72 0 0 cm /Im0 Do Q", nestedSecondPage: nested,
                        gray: isGray, pageColorEntries: pageColors, formColorEntries: formColors).write(to: input)
            _ = try OrganizeEngine(qpdf: qpdf, scratch: root).compress(input, output: output, preset: .smallSize)
            XCTAssertEqual(try imageDimensions(output).width, 1024, "Custom default spaces must exclude the shared image")
            XCTAssertEqual(try imageStreamBytes(output), try imageStreamBytes(input))
        }
    }

    func testWideGamutJPEGProfileKeepsOriginalImageBytes() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("p3.pdf"), output = root.appendingPathComponent("result.pdf")
        try fixture(jpeg: true, jpegColorSpace: CGColorSpace(name: CGColorSpace.displayP3)).write(to: input)
        let bytes = try imageStreamBytes(input)
        let source = try XCTUnwrap(CGImageSourceCreateWithData(bytes as CFData, nil))
        let image = try XCTUnwrap(CGImageSourceCreateImageAtIndex(source, 0, nil))
        XCTAssertEqual(image.colorSpace?.name, CGColorSpace.displayP3)
        _ = try OrganizeEngine(qpdf: qpdf, scratch: root).compress(input, output: output, preset: .smallSize)
        XCTAssertEqual(try imageDimensions(output).width, 1024)
        XCTAssertEqual(try imageStreamBytes(output), bytes)
    }

    func testAnnotationAppearancesPreserveSharedImagesButDoNotBlockUnrelatedPageImages() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        for shared in [false, true] {
            let input = root.appendingPathComponent("appearance-\(shared).pdf"), output = root.appendingPathComponent("output-\(shared).pdf")
            try fixture(jpeg: true, appearanceImage: shared).write(to: input)
            _ = try OrganizeEngine(qpdf: qpdf, scratch: root).compress(input, output: output, preset: .smallSize)
            XCTAssertEqual(try imageDimensions(output).width, shared ? 1024 : 540,
                "Only images reachable through an unscanned annotation appearance must be excluded")
            let document = try XCTUnwrap(PDFDocument(url: output))
            XCTAssertEqual(document.page(at: 0)?.annotations.count, 2)
        }
    }

    func testCyclicFormsAndOversizedImageMetadataAreConservativelySkipped() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("cycle.pdf"), output = root.appendingPathComponent("cycle-output.pdf")
        try fixture(jpeg: true, nestedSecondPage: true, cyclicForm: true).write(to: input)
        _ = try OrganizeEngine(qpdf: qpdf, scratch: root).compress(input, output: output, preset: .smallSize)
        XCTAssertEqual(try imageDimensions(output).width, 1024)

        let source = root.appendingPathComponent("budget.pdf")
        try fixture(jpeg: true).write(to: source)
        var inspection = try inspect(source)
        var entries = try XCTUnwrap(inspection["qpdf"] as? [Any])
        var objects = try XCTUnwrap(entries[1] as? [String: Any])
        var image = try XCTUnwrap(objects["obj:5 0 R"] as? [String: Any])
        var stream = try XCTUnwrap(image["stream"] as? [String: Any])
        var dictionary = try XCTUnwrap(stream["dict"] as? [String: Any])
        dictionary["/Width"] = 20_000; dictionary["/Height"] = 20_000
        stream["dict"] = dictionary; image["stream"] = stream; objects["obj:5 0 R"] = image
        entries[1] = objects; inspection["qpdf"] = entries
        var didExtract = false
        let result = try PDFImageCompression.prepare(input: source, inspection: inspection, job: root, dpi: 72, quality: 40,
            cancelled: { false }, run: { _, _, _ in didExtract = true; return Data() })
        XCTAssertNil(result)
        XCTAssertFalse(didExtract, "Unsafe dimensions must be rejected before any image data decode/allocation")
    }

    func testCancellationAfterImageExtractionLeavesNoOutputOrScratchFiles() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("source.pdf"), output = root.appendingPathComponent("result.pdf")
        let source = try fixture(jpeg: true); try source.write(to: input)
        var reachedImageProcessing = false
        XCTAssertThrowsError(try OrganizeEngine(qpdf: qpdf, scratch: root).compress(input, output: output, preset: .smallSize, cancelled: {
            let enumerator = FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil)
            while let file = enumerator?.nextObject() as? URL {
                if file.lastPathComponent.hasSuffix("-source") { reachedImageProcessing = true; return true }
            }
            return false
        })) { XCTAssertEqual($0 as? OrganizeError, .cancelled) }
        XCTAssertTrue(reachedImageProcessing)
        XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: root.path), ["source.pdf"])
        XCTAssertEqual(try Data(contentsOf: input), source)
    }

    func testCompressedRenderingAndOriginalPageOperatorsArePreserved() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("source.pdf"), output = root.appendingPathComponent("result.pdf")
        try fixture(jpeg: true, width: 1800, height: 1200).write(to: input)
        let result = try OrganizeEngine(qpdf: qpdf, scratch: root).compress(input, output: output, preset: .balanced)
        let before = try renderedPixels(input), after = try renderedPixels(output)
        XCTAssertEqual(before.count, after.count)
        let absoluteError = zip(before, after).reduce(0.0) { $0 + abs(Double($1.0) - Double($1.1)) } / Double(before.count)
        XCTAssertLessThan(absoluteError, 12, "At 72dpi the 150dpi compressed page must remain recognizably faithful, not blank, mirrored, or inverted")
        XCTAssertEqual(try pageOperators(input), try pageOperators(output), "Image compression must not replace or rewrite page painting/text operators")
        print("COMPRESSION EVIDENCE balanced: \(result.before) -> \(result.after) bytes; render MAE=\(absoluteError)")
        if let destination = ProcessInfo.processInfo.environment["PDFMASTER_COMPRESSION_EVIDENCE_DIR"] {
            let folder = URL(fileURLWithPath: destination, isDirectory: true)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try FileManager.default.copyItem(at: input, to: folder.appendingPathComponent("source.pdf"))
            try FileManager.default.copyItem(at: output, to: folder.appendingPathComponent("compressed.pdf"))
            try renderPNG(input, to: folder.appendingPathComponent("source.png"))
            try renderPNG(output, to: folder.appendingPathComponent("compressed.png"))
        }
    }

    func testImageCompressionPreservesTextLinksAnnotationsAndRotation() throws {
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("source.pdf"), output = root.appendingPathComponent("compressed.pdf")
        let source = try fixture(); try source.write(to: input)
        let engine = try OrganizeEngine(qpdf: qpdf, scratch: root)
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
        XCTAssertEqual(PDFCompressionPreset.allCases.count, 4)
        XCTAssertEqual(PDFCompressionPreset.allCases.map(\.jpegQuality), [nil, 85, 65, 40])
        XCTAssertEqual(Set(PDFCompressionPreset.allCases.map(\.title)).count, 4)
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("source.pdf"), source = try fixture(); try source.write(to: input)
        let engine = try OrganizeEngine(qpdf: qpdf, scratch: root)
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
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("source.pdf"), output = root.appendingPathComponent("unchanged.pdf")
        let source = pdf(objects: [Data("<< /Type /Catalog /Pages 2 0 R >>".utf8),
                                  Data("<< /Type /Pages /Kids [3 0 R] /Count 1 >>".utf8),
                                  Data("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> >>".utf8)])
        try source.write(to: input)
        let engine = try OrganizeEngine(qpdf: qpdf, scratch: root)
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
        let root = try directory(); defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("signed.pdf"), output = root.appendingPathComponent("result.pdf")
        try fixture(signed: true).write(to: input)
        let engine = try OrganizeEngine(qpdf: qpdf, scratch: root)
        XCTAssertThrowsError(try engine.validateCompressionInput(input)) {
            XCTAssertEqual($0 as? OrganizeError, .signed)
        }
        XCTAssertThrowsError(try engine.compress(input, output: output, preset: .balanced)) {
            XCTAssertEqual($0 as? OrganizeError, .signed)
        }
        let plain = root.appendingPathComponent("plain.pdf"), encrypted = root.appendingPathComponent("encrypted.pdf")
        try fixture().write(to: plain)
        let process = Process(); process.executableURL = try qpdf
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

    private func imageDimensions(_ url: URL) throws -> (width: Int, height: Int) {
        let document = try XCTUnwrap(CGPDFDocument(url as CFURL)), page = try XCTUnwrap(document.page(at: 1))
        let dictionary = try XCTUnwrap(page.dictionary)
        var resources: CGPDFDictionaryRef?, xobjects: CGPDFDictionaryRef?, stream: CGPDFStreamRef?
        XCTAssertTrue(CGPDFDictionaryGetDictionary(dictionary, "Resources", &resources))
        XCTAssertTrue(CGPDFDictionaryGetDictionary(try XCTUnwrap(resources), "XObject", &xobjects))
        XCTAssertTrue(CGPDFDictionaryGetStream(try XCTUnwrap(xobjects), "Im0", &stream))
        let image = try XCTUnwrap(CGPDFStreamGetDictionary(try XCTUnwrap(stream)))
        var width = 0, height = 0
        XCTAssertTrue(CGPDFDictionaryGetInteger(image, "Width", &width))
        XCTAssertTrue(CGPDFDictionaryGetInteger(image, "Height", &height))
        return (width, height)
    }

    private func imageStreamBytes(_ url: URL) throws -> Data {
        let document = try XCTUnwrap(CGPDFDocument(url as CFURL)), page = try XCTUnwrap(document.page(at: 1))
        var resources: CGPDFDictionaryRef?, xobjects: CGPDFDictionaryRef?, stream: CGPDFStreamRef?
        XCTAssertTrue(CGPDFDictionaryGetDictionary(try XCTUnwrap(page.dictionary), "Resources", &resources))
        XCTAssertTrue(CGPDFDictionaryGetDictionary(try XCTUnwrap(resources), "XObject", &xobjects))
        XCTAssertTrue(CGPDFDictionaryGetStream(try XCTUnwrap(xobjects), "Im0", &stream))
        var format = CGPDFDataFormat.raw
        return try XCTUnwrap(CGPDFStreamCopyData(try XCTUnwrap(stream), &format)) as Data
    }

    private func inspect(_ url: URL) throws -> [String: Any] {
        let process = Process(), pipe = Pipe()
        process.executableURL = try qpdf
        process.arguments = ["--json", "--json-key=qpdf", "--json-key=pages", "--json-stream-data=none", url.path]
        process.standardOutput = pipe
        try process.run()
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit(); XCTAssertEqual(process.terminationStatus, 0)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    private func pageOperators(_ url: URL) throws -> Data {
        let document = try XCTUnwrap(CGPDFDocument(url as CFURL)), page = try XCTUnwrap(document.page(at: 1))
        var stream: CGPDFStreamRef?
        XCTAssertTrue(CGPDFDictionaryGetStream(try XCTUnwrap(page.dictionary), "Contents", &stream))
        var format = CGPDFDataFormat.raw
        return try XCTUnwrap(CGPDFStreamCopyData(try XCTUnwrap(stream), &format)) as Data
    }

    private func renderedImage(_ url: URL) throws -> CGImage {
        let document = try XCTUnwrap(CGPDFDocument(url as CFURL)), page = try XCTUnwrap(document.page(at: 1))
        let context = try XCTUnwrap(CGContext(data: nil, width: 400, height: 600, bitsPerComponent: 8,
            bytesPerRow: 1600, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
        context.setFillColor(CGColor(gray: 1, alpha: 1)); context.fill(CGRect(x: 0, y: 0, width: 400, height: 600))
        context.concatenate(page.getDrawingTransform(.mediaBox, rect: CGRect(x: 0, y: 0, width: 400, height: 600), rotate: 0, preserveAspectRatio: true))
        context.drawPDFPage(page)
        return try XCTUnwrap(context.makeImage())
    }

    private func renderedPixels(_ url: URL) throws -> Data {
        let image = try renderedImage(url)
        return try XCTUnwrap(image.dataProvider?.data) as Data
    }

    private func renderPNG(_ url: URL, to output: URL) throws {
        let destination = try XCTUnwrap(CGImageDestinationCreateWithURL(output as CFURL, UTType.png.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(destination, try renderedImage(url), nil)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
    }

    private func fixture(signed: Bool = false, jpeg: Bool = false, width: Int = 1024, height: Int = 768,
                         drawing: String? = nil, nestedSecondPage: Bool = false, cyclicForm: Bool = false,
                         appearanceImage: Bool? = nil, extraImageEntries: String = "", gray: Bool = false,
                         pageColorEntries: String = "", formColorEntries: String = "", jpegColorSpace: CGColorSpace? = nil,
                         repeatedPages: Int = 1, imageCopies: Int = 1,
                         secondPageRepeats: Int = 1, innerFormOperations: Int = 0) throws -> Data {
        let channels = gray ? 1 : 3
        var pixels = Data(count: width * height * channels)
        pixels.withUnsafeMutableBytes { raw in
            let bytes = raw.bindMemory(to: UInt8.self)
            var seed: UInt32 = 7
            for y in 0..<height { for x in 0..<width {
                seed = 1664525 &* seed &+ 1013904223
                let noise = Int((seed >> 24) & 31)
                let i = (y * width + x) * channels
                bytes[i] = UInt8((x / 5 + noise) % 256)
                if !gray {
                    bytes[i + 1] = UInt8((y / 4 + noise) % 256)
                    bytes[i + 2] = UInt8(((x + y) / 8 + noise) % 256)
                }
            } }
        }
        var image: Data
        if jpeg {
            let provider = try XCTUnwrap(CGDataProvider(data: pixels as CFData))
            let cgImage = try XCTUnwrap(CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: channels * 8,
                bytesPerRow: width * channels, space: jpegColorSpace ?? (gray ? CGColorSpaceCreateDeviceGray() : CGColorSpaceCreateDeviceRGB()), bitmapInfo: CGBitmapInfo(rawValue: 0),
                provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent))
            let data = NSMutableData()
            let destination = try XCTUnwrap(CGImageDestinationCreateWithData(data, UTType.jpeg.identifier as CFString, 1, nil))
            CGImageDestinationAddImage(destination, cgImage, [kCGImageDestinationLossyCompressionQuality: 0.98] as CFDictionary)
            XCTAssertTrue(CGImageDestinationFinalize(destination))
            image = data as Data
        } else {
        let deflate = try (pixels as NSData).compressed(using: .zlib) as Data
        var a: UInt32 = 1, b: UInt32 = 0
        for byte in pixels { a = (a + UInt32(byte)) % 65521; b = (b + a) % 65521 }
        let checksum = (b << 16) | a
        image = Data([0x78, 0x9c]); image.append(deflate)
        image.append(contentsOf: [UInt8((checksum >> 24) & 255), UInt8((checksum >> 16) & 255), UInt8((checksum >> 8) & 255), UInt8(checksum & 255)])
        }
        func stream(_ header: String, _ data: Data) -> Data {
            var value = Data("<< \(header) /Length \(data.count) >>\nstream\n".utf8)
            value.append(data); value.append(Data("\nendstream".utf8)); return value
        }
        var objects: [Data] = [
            Data("<< /Type /Catalog /Pages 2 0 R >>".utf8),
            Data("<< /Type /Pages /Kids [3 0 R \((1..<repeatedPages).map { "\(8+$0) 0 R" }.joined(separator: " ")) \(nestedSecondPage ? "9 0 R" : "")] /Count \(nestedSecondPage ? 2 : repeatedPages) >>".utf8),
            Data("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 400] /Rotate 90 /Resources << /XObject << /Im0 5 0 R \((1..<imageCopies).map { "/Im\($0) \(8+$0) 0 R" }.joined(separator: " ")) >> /Font << /F1 6 0 R >> \(pageColorEntries) >> /Contents 4 0 R /Annots [7 0 R 8 0 R] >>".utf8),
            stream("", Data("\(drawing ?? "q 540 0 0 280 30 20 cm /Im0 Do Q")\n\((1..<imageCopies).map { "q 72 0 0 72 0 0 cm /Im\($0) Do Q" }.joined(separator: "\n"))\nBT /F1 18 Tf 30 350 Td (SEARCHABLE COMPRESSION TEST) Tj ET".utf8)),
            stream("/Type /XObject /Subtype /Image /Width \(width) /Height \(height) /ColorSpace \(gray ? "/DeviceGray" : "/DeviceRGB") /BitsPerComponent 8 /Filter \(jpeg ? "/DCTDecode" : "/FlateDecode") \(extraImageEntries)", image),
            Data("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".utf8),
            Data("<< /Type /Annot /Subtype /Link /Rect [30 310 200 330] /A << /S /URI /URI (https://example.com/pdf) >> >>".utf8),
            Data("<< /Type /Annot /Subtype /Highlight /Rect [30 345 250 365] /QuadPoints [30 365 250 365 30 345 250 345] /C [1 1 0] /Contents (Keep this comment) \(appearanceImage == nil ? "" : "/AP << /N 9 0 R >>") >>".utf8)
        ]
        for _ in 1..<repeatedPages { objects.append(objects[2]) }
        for _ in 1..<imageCopies {
            objects.append(stream("/Type /XObject /Subtype /Image /Width \(width) /Height \(height) /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode", image))
        }
        if let appearanceImage {
            objects.append(stream("/Type /XObject /Subtype /Form /BBox [0 0 220 20] /Resources << /XObject << \(appearanceImage ? "/Im0 5 0 R" : "") >> >>",
                Data((appearanceImage ? "220 0 0 20 0 0 cm /Im0 Do" : "1 1 0 rg 0 0 220 20 re f").utf8)))
        }
        if nestedSecondPage {
            objects += [
                Data("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 400] /Rotate 270 /Resources << /XObject << /Outer 11 0 R >> >> /Contents 10 0 R >>".utf8),
                stream("", Data(String(repeating: "q 108 0 0 72 0 0 cm /Outer Do Q\n", count: secondPageRepeats).utf8)),
                stream("/Type /XObject /Subtype /Form /BBox [0 0 3 3] /Matrix [0 1 -1 0 0 0] /Resources << /XObject << /Inner 12 0 R >> \(formColorEntries) >>", Data("/Inner Do".utf8)),
                stream("/Type /XObject /Subtype /Form /BBox [0 0 1 1] /Matrix [2 0 0 3 0 0] /Resources << /XObject << \(cyclicForm ? "/Loop 11 0 R" : "/Im0 5 0 R") >> >>", Data((String(repeating: "n\n", count: innerFormOperations) + (cyclicForm ? "/Loop Do" : "/Im0 Do")).utf8))
            ]
        }
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
