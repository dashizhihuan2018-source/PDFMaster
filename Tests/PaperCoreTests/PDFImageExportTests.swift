import CoreGraphics
import ImageIO
import PDFKit
import XCTest
@testable import PaperCore

final class PDFImageExportTests: XCTestCase {
    func testExportsSelectedPagesAsPNGAtRequestedDPI() throws {
        let folder = temporaryDirectory()
        let destination = folder.appendingPathComponent("selected-images", isDirectory: true)
        let request = PDFImageExportRequest(format: .png, dpi: 144, pages: IndexSet([0, 2]))

        let result = try PDFImageExport.export(
            data: try fixturePDF(pageSizes: [CGSize(width: 100, height: 80), CGSize(width: 60, height: 50), CGSize(width: 40, height: 30)]),
            to: destination,
            request: request
        )

        XCTAssertEqual(result.pageNumbers, [1, 3])
        XCTAssertEqual(result.files.map(\.lastPathComponent), ["page-0001.png", "page-0003.png"])
        XCTAssertEqual(try pixelSize(result.files[0]), CGSize(width: 200, height: 160))
        XCTAssertEqual(try pixelSize(result.files[1]), CGSize(width: 80, height: 60))
        XCTAssertTrue(FileManager.default.fileExists(atPath: destination.path))
    }

    func testExportsJPEGAndRecordsRequestedResolution() throws {
        let destination = temporaryDirectory().appendingPathComponent("jpeg-images", isDirectory: true)
        let result = try PDFImageExport.export(
            data: try fixturePDF(pageSizes: [CGSize(width: 72, height: 36)]),
            to: destination,
            request: PDFImageExportRequest(format: .jpeg, dpi: 300, pages: IndexSet(integer: 0), jpegQuality: 0.82)
        )

        XCTAssertEqual(result.files.map(\.pathExtension), ["jpg"])
        XCTAssertEqual(try pixelSize(result.files[0]), CGSize(width: 300, height: 150))
        let source = try XCTUnwrap(CGImageSourceCreateWithURL(result.files[0] as CFURL, nil))
        let properties = try XCTUnwrap(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
        XCTAssertEqual((properties[kCGImagePropertyDPIWidth] as? NSNumber)?.doubleValue ?? 0, 300, accuracy: 0.5)
    }

    func testExistingDestinationIsNeverOverwritten() throws {
        let folder = temporaryDirectory()
        let destination = folder.appendingPathComponent("existing", isDirectory: true)
        try FileManager.default.createDirectory(at: destination, withIntermediateDirectories: false)
        let sentinel = destination.appendingPathComponent("keep.txt")
        try Data("keep".utf8).write(to: sentinel)

        XCTAssertThrowsError(try PDFImageExport.export(
            data: try fixturePDF(pageSizes: [CGSize(width: 20, height: 20)]),
            to: destination,
            request: PDFImageExportRequest(format: .png, dpi: 72, pages: IndexSet(integer: 0))
        )) { XCTAssertEqual($0 as? PDFImageExportFailure, .outputExists) }
        XCTAssertEqual(try String(contentsOf: sentinel, encoding: .utf8), "keep")
    }

    func testCancellationLeavesNoPublishedOrStagingDirectory() throws {
        let folder = temporaryDirectory()
        let destination = folder.appendingPathComponent("cancelled", isDirectory: true)
        var checks = 0
        XCTAssertThrowsError(try PDFImageExport.export(
            data: try fixturePDF(pageSizes: [CGSize(width: 40, height: 40), CGSize(width: 40, height: 40)]),
            to: destination,
            request: PDFImageExportRequest(format: .png, dpi: 72, pages: IndexSet([0, 1])),
            cancelled: { checks += 1; return checks >= 3 }
        )) { XCTAssertEqual($0 as? PDFImageExportFailure, .cancelled) }
        XCTAssertFalse(FileManager.default.fileExists(atPath: destination.path))
        XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: folder.path).allSatisfy { !$0.hasPrefix(".pdf-image-export-") })
    }

    func testRejectsInvalidDPISelectionAndUnreadableInputWithoutPublishing() throws {
        let folder = temporaryDirectory()
        for (name, data, request) in [
            ("dpi", try fixturePDF(pageSizes: [CGSize(width: 20, height: 20)]), PDFImageExportRequest(format: .png, dpi: 12, pages: IndexSet(integer: 0))),
            ("page", try fixturePDF(pageSizes: [CGSize(width: 20, height: 20)]), PDFImageExportRequest(format: .png, dpi: 72, pages: IndexSet(integer: 2))),
            ("bad", Data(), PDFImageExportRequest(format: .png, dpi: 72, pages: IndexSet(integer: 0)))
        ] {
            let destination = folder.appendingPathComponent(name, isDirectory: true)
            XCTAssertThrowsError(try PDFImageExport.export(data: data, to: destination, request: request))
            XCTAssertFalse(FileManager.default.fileExists(atPath: destination.path))
        }
    }

    func testExtremePageBoxesFailBeforeIntegerConversionAndLeaveNoArtifacts() throws {
        let folder = temporaryDirectory()
        let destination = folder.appendingPathComponent("extreme", isDirectory: true)
        let data = try extremeBoundsPDF()

        XCTAssertThrowsError(try PDFImageExport.export(
            data: data,
            to: destination,
            request: PDFImageExportRequest(format: .png, dpi: 600, pages: IndexSet(integer: 0))
        )) { XCTAssertEqual($0 as? PDFImageExportFailure, .tooLarge) }
        XCTAssertFalse(FileManager.default.fileExists(atPath: destination.path))
        XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: folder.path).allSatisfy { !$0.hasPrefix(".pdf-image-export-") })
    }

    func testDefaultPixelBudgetsBoundOnePageAndWholeJobMemory() {
        let request = PDFImageExportRequest(format: .png, dpi: 144, pages: IndexSet(integer: 0))
        XCTAssertEqual(request.maximumPixelsPerPage, 32_000_000)
        XCTAssertEqual(request.maximumTotalPixels, 100_000_000)
    }

    private func fixturePDF(pageSizes: [CGSize]) throws -> Data {
        let data = NSMutableData()
        guard let consumer = CGDataConsumer(data: data as CFMutableData),
              let context = CGContext(consumer: consumer, mediaBox: nil, nil) else { throw CocoaError(.fileWriteUnknown) }
        for (index, size) in pageSizes.enumerated() {
            var box = CGRect(origin: .zero, size: size)
            context.beginPDFPage([kCGPDFContextMediaBox: Data(bytes: &box, count: MemoryLayout<CGRect>.size)] as CFDictionary)
            context.setFillColor(CGColor(red: CGFloat(index + 1) / CGFloat(pageSizes.count + 1), green: 0.3, blue: 0.7, alpha: 1))
            context.fill(box)
            context.endPDFPage()
        }
        context.closePDF()
        return data as Data
    }

    private func extremeBoundsPDF() throws -> Data {
        let document = PDFDocument()
        let image = NSImage(size: NSSize(width: 10, height: 10))
        image.lockFocus(); NSColor.white.setFill(); NSRect(x: 0, y: 0, width: 10, height: 10).fill(); image.unlockFocus()
        let page = try XCTUnwrap(PDFPage(image: image))
        let extreme = CGRect(x: 0, y: 0, width: 1e20, height: 1e20)
        page.setBounds(extreme, for: .mediaBox)
        page.setBounds(extreme, for: .cropBox)
        document.insert(page, at: 0)
        return try XCTUnwrap(document.dataRepresentation())
    }

    private func pixelSize(_ url: URL) throws -> CGSize {
        let source = try XCTUnwrap(CGImageSourceCreateWithURL(url as CFURL, nil))
        let properties = try XCTUnwrap(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
        return CGSize(width: try XCTUnwrap(properties[kCGImagePropertyPixelWidth] as? NSNumber).doubleValue,
                      height: try XCTUnwrap(properties[kCGImagePropertyPixelHeight] as? NSNumber).doubleValue)
    }

    private func temporaryDirectory() -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("PDFImageExportTests-\(UUID().uuidString)", isDirectory: true)
        try! FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }
}
