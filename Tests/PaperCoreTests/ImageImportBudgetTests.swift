import XCTest
import CoreGraphics
import ImageIO
@testable import PaperCore

final class ImageImportBudgetTests: XCTestCase {
    func testDefaultsPreservePageCeilingAndSetBatchAndOutputBudgets() {
        let limits = ImageImportLimits()
        XCTAssertEqual(limits.maximumDecodedPixelsPerFrame, 40_000_000)
        XCTAssertEqual(limits.maximumTotalDecodedPixels, 100_000_000)
        XCTAssertEqual(limits.maximumPages, 500)
        XCTAssertEqual(limits.maximumOutputBytes, 256 * 1024 * 1024)
    }

    func testPixelArithmeticRejectsNonFiniteFractionalAndOverflowingMetadata() throws {
        XCTAssertEqual(try ImagePDFImport.checkedPixelCount(width: 4, height: 3, maximum: 12), 12)
        for (width, height) in [(Double.nan, 1.0), (Double.infinity, 1.0), (1.0, Double.infinity),
                                (0.0, 1.0), (-1.0, 1.0), (1.5, 1.0),
                                (Double(UInt64.max), 1.0), (Double(UInt64.max - 4096), 2.0)] {
            XCTAssertThrowsError(try ImagePDFImport.checkedPixelCount(width: width, height: height, maximum: UInt64.max)) {
                XCTAssertEqual($0 as? ImportFailure, .tooLarge)
            }
        }
        XCTAssertThrowsError(try ImagePDFImport.checkedPixelCount(width: 4, height: 3, maximum: 11)) {
            XCTAssertEqual($0 as? ImportFailure, .tooLarge)
        }
        XCTAssertEqual(try ImagePDFImport.checkedTotal(12, adding: 20, maximum: 32), 32)
        XCTAssertThrowsError(try ImagePDFImport.checkedTotal(UInt64.max, adding: 1, maximum: UInt64.max)) {
            XCTAssertEqual($0 as? ImportFailure, .tooLarge)
        }
    }

    func testDefault500PageCeilingIncludesTIFFFramesBeforeCreatingStage() throws {
        let root = try makeRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let png = try makePNG(in: root, name: "pixel", width: 1, height: 1)
        let source = try XCTUnwrap(CGImageSourceCreateWithURL(png as CFURL, nil))
        let image = try XCTUnwrap(CGImageSourceCreateImageAtIndex(source, 0, nil))
        let tiff = root.appendingPathComponent("501.tiff")
        let writer = try XCTUnwrap(CGImageDestinationCreateWithURL(tiff as CFURL, "public.tiff" as CFString, 501, nil))
        for _ in 0..<501 { CGImageDestinationAddImage(writer, image, nil) }
        XCTAssertTrue(CGImageDestinationFinalize(writer))
        let original = try Data(contentsOf: tiff)
        let output = root.appendingPathComponent("output.pdf")
        var observedStage = false
        XCTAssertThrowsError(try ImagePDFImport.convert([tiff], to: output, cancelled: {
            observedStage = observedStage || self.hasStage(in: root)
            return false
        })) { XCTAssertEqual($0 as? ImportFailure, .tooLarge) }
        XCTAssertFalse(observedStage)
        XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
        XCTAssertEqual(try Data(contentsOf: tiff), original)
    }

    func testCumulativePixelBudgetRejectsBeforeAnyOutputStage() throws {
        let root = try makeRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let inputs = try [makePNG(in: root, name: "first", width: 4, height: 3), makePNG(in: root, name: "second", width: 5, height: 4)]
        let originals = try inputs.map { try Data(contentsOf: $0) }
        let output = root.appendingPathComponent("output.pdf")
        var observedStage = false
        XCTAssertThrowsError(try ImagePDFImport.convert(inputs, to: output, limits: .init(maximumTotalDecodedPixels: 31), cancelled: {
            observedStage = observedStage || self.hasStage(in: root)
            return false
        })) { XCTAssertEqual($0 as? ImportFailure, .tooLarge) }
        XCTAssertFalse(observedStage)
        XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
        XCTAssertEqual(try inputs.map { try Data(contentsOf: $0) }, originals)
        XCTAssertFalse(hasStage(in: root))
    }

    func testFrameBudgetRejectsBeforeDecodeAndLeavesSourceUnchanged() throws {
        let root = try makeRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let input = try makePNG(in: root, name: "first", width: 4, height: 3)
        let original = try Data(contentsOf: input)
        let output = root.appendingPathComponent("output.pdf")
        var observedStage = false
        XCTAssertThrowsError(try ImagePDFImport.convert([input], to: output, limits: .init(maximumDecodedPixelsPerFrame: 11), cancelled: {
            observedStage = observedStage || self.hasStage(in: root)
            return false
        })) { XCTAssertEqual($0 as? ImportFailure, .tooLarge) }
        XCTAssertFalse(observedStage)
        XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
        XCTAssertEqual(try Data(contentsOf: input), original)
    }

    func testTIFFFramesCountTowardsBatchPixelAndPageBudgets() throws {
        let root = try makeRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let png = try makePNG(in: root, name: "first", width: 4, height: 3)
        let source = try XCTUnwrap(CGImageSourceCreateWithURL(png as CFURL, nil))
        let image = try XCTUnwrap(CGImageSourceCreateImageAtIndex(source, 0, nil))
        let tiff = root.appendingPathComponent("three.tiff")
        let writer = try XCTUnwrap(CGImageDestinationCreateWithURL(tiff as CFURL, "public.tiff" as CFString, 3, nil))
        for _ in 0..<3 { CGImageDestinationAddImage(writer, image, nil) }
        XCTAssertTrue(CGImageDestinationFinalize(writer))
        let original = try Data(contentsOf: tiff)
        for (index, limits) in [ImageImportLimits(maximumTotalDecodedPixels: 35), ImageImportLimits(maximumPages: 2)].enumerated() {
            let output = root.appendingPathComponent("output-\(index).pdf")
            var observedStage = false
            XCTAssertThrowsError(try ImagePDFImport.convert([tiff], to: output, limits: limits, cancelled: {
                observedStage = observedStage || self.hasStage(in: root)
                return false
            })) { XCTAssertEqual($0 as? ImportFailure, .tooLarge) }
            XCTAssertFalse(observedStage)
            XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
            XCTAssertFalse(hasStage(in: root))
            XCTAssertEqual(try Data(contentsOf: tiff), original)
        }
    }

    func testOutputBudgetDeletesStageAndDoesNotPublishOrModifySource() throws {
        let root = try makeRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let input = try makePNG(in: root, name: "first", width: 4, height: 3)
        let original = try Data(contentsOf: input)
        let output = root.appendingPathComponent("output.pdf")
        XCTAssertThrowsError(try ImagePDFImport.convert([input], to: output, limits: .init(maximumOutputBytes: 64))) {
            XCTAssertEqual($0 as? ImportFailure, .tooLarge)
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
        XCTAssertFalse(hasStage(in: root))
        XCTAssertEqual(try Data(contentsOf: input), original)
    }

    func testInvalidOrRaisedHardLimitsCannotCreateOutput() throws {
        let root = try makeRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let input = try makePNG(in: root, name: "first", width: 4, height: 3)
        for limits in [ImageImportLimits(maximumDecodedPixelsPerFrame: 0),
                       ImageImportLimits(maximumTotalDecodedPixels: 0),
                       ImageImportLimits(maximumPages: 0), ImageImportLimits(maximumPages: 501),
                       ImageImportLimits(maximumOutputBytes: -1),
                       ImageImportLimits(maximumDecodedPixelsPerFrame: UInt64.max),
                       ImageImportLimits(maximumTotalDecodedPixels: UInt64.max),
                       ImageImportLimits(maximumOutputBytes: Int.max)] {
            let output = root.appendingPathComponent("output.pdf")
            XCTAssertThrowsError(try ImagePDFImport.convert([input], to: output, limits: limits)) {
                XCTAssertEqual($0 as? ImportFailure, .invalidInput)
            }
            // Avoid masking subsequent failures if an old implementation incorrectly publishes.
            if FileManager.default.fileExists(atPath: output.path) { try FileManager.default.removeItem(at: output) }
            XCTAssertFalse(hasStage(in: root))
        }
    }

    func testExactPixelBudgetStillProducesStandardPages() throws {
        let root = try makeRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let inputs = try [makePNG(in: root, name: "first", width: 4, height: 3), makePNG(in: root, name: "second", width: 5, height: 4)]
        let originals = try inputs.map { try Data(contentsOf: $0) }
        let output = root.appendingPathComponent("output.pdf")
        XCTAssertEqual(try ImagePDFImport.convert(inputs, to: output, limits: .init(maximumTotalDecodedPixels: 32, maximumPages: 2)), 2)
        let pdf = try XCTUnwrap(CGPDFDocument(output as CFURL))
        XCTAssertEqual(pdf.numberOfPages, 2)
        XCTAssertEqual(pdf.page(at: 1)?.getBoxRect(.mediaBox).width ?? 0, 595.276, accuracy: 0.001)
        XCTAssertEqual(try inputs.map { try Data(contentsOf: $0) }, originals)
        XCTAssertFalse(hasStage(in: root))
    }

    func testAllInputsArePreflightedBeforeAnyPDFStageIsCreated() throws {
        let root = try makeRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let image = try makePNG(in: root, name: "first", width: 4, height: 3)
        let original = try Data(contentsOf: image)
        let output = root.appendingPathComponent("output.pdf")
        var observedStage = false
        XCTAssertThrowsError(try ImagePDFImport.convert([image, root.appendingPathComponent("missing.png")], to: output, cancelled: {
            observedStage = observedStage || ((try? FileManager.default.contentsOfDirectory(atPath: root.path)) ?? []).contains { $0.hasPrefix(".image-import-") }
            return false
        })) { XCTAssertEqual($0 as? ImportFailure, .invalidInput) }
        XCTAssertFalse(observedStage, "Reject the entire metadata batch before creating any partial PDF")
        XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
        XCTAssertEqual(try Data(contentsOf: image), original)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: root.path), ["first.png"])
    }

    private func makeRoot() throws -> URL {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-image-budget-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        return root
    }

    private func hasStage(in root: URL) -> Bool {
        ((try? FileManager.default.contentsOfDirectory(atPath: root.path)) ?? []).contains { $0.hasPrefix(".image-import-") }
    }

    private func makePNG(in root: URL, name: String, width: Int, height: Int) throws -> URL {
        let context = try XCTUnwrap(CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                                             space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.setFillColor(CGColor(gray: 0.4, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        let image = try XCTUnwrap(context.makeImage())
        let url = root.appendingPathComponent(name + ".png")
        let writer = try XCTUnwrap(CGImageDestinationCreateWithURL(url as CFURL, "public.png" as CFString, 1, nil))
        CGImageDestinationAddImage(writer, image, nil)
        XCTAssertTrue(CGImageDestinationFinalize(writer))
        return url
    }
}
