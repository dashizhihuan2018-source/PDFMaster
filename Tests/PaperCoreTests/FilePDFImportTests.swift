import XCTest
import PDFKit
import AppKit
import ImageIO
@testable import PaperCore

final class FilePDFImportTests: XCTestCase {
    @MainActor func testInsertedImagesKeepWideTallAndSquarePagesBesideExistingDocument() throws {
        let dir = try directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let sizes = [CGSize(width: 80, height: 40), CGSize(width: 40, height: 80), CGSize(width: 60, height: 60)]
        let images = try sizes.enumerated().map { index, size in
            try image(dir, name: "image-\(index).png", width: Int(size.width), height: Int(size.height))
        }
        let originalImages = try images.map { try Data(contentsOf: $0) }
        let workspace = PDFWorkspace(); try workspace.createBlank()
        let existingBounds = try XCTUnwrap(workspace.document?.page(at: 0)?.bounds(for: .mediaBox))
        try workspace.insertPDFData(FilePDFImport.prepare(images), at: 1)
        XCTAssertEqual(workspace.document?.pageCount, 4)
        XCTAssertEqual(workspace.document?.page(at: 0)?.bounds(for: .mediaBox), existingBounds)
        let output = dir.appendingPathComponent("inserted.pdf"); try workspace.saveCopy(to: output)
        let reopened = try XCTUnwrap(PDFDocument(url: output))
        for (index, size) in sizes.enumerated() {
            XCTAssertEqual(workspace.document?.page(at: index + 1)?.bounds(for: .mediaBox).size, size)
            XCTAssertEqual(reopened.page(at: index + 1)?.bounds(for: .mediaBox).size, size)
        }
        try workspace.undo()
        XCTAssertEqual(workspace.document?.pageCount, 1)
        XCTAssertEqual(workspace.document?.page(at: 0)?.bounds(for: .mediaBox), existingBounds)
        XCTAssertEqual(try images.map { try Data(contentsOf: $0) }, originalImages)
    }

    func testPhoneImageBatchCanExceedOneHundredMegapixelsWithoutChangingSources() throws {
        let dir = try directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let context = try XCTUnwrap(CGContext(data: nil, width: 4000, height: 3000, bitsPerComponent: 8, bytesPerRow: 16000,
                                            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
        context.setFillColor(CGColor(red: 0.1, green: 0.3, blue: 0.8, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 4000, height: 3000))
        let input = dir.appendingPathComponent("phone.png")
        let writer = try XCTUnwrap(CGImageDestinationCreateWithURL(input as CFURL, "public.png" as CFString, 1, nil))
        CGImageDestinationAddImage(writer, try XCTUnwrap(context.makeImage()), nil)
        XCTAssertTrue(CGImageDestinationFinalize(writer))
        let original = try Data(contentsOf: input)
        let batch = try FilePDFImport.prepare(Array(repeating: input, count: 9))
        XCTAssertEqual(batch.count, 9)
        XCTAssertEqual(batch.compactMap { PDFDocument(data: $0)?.pageCount }, Array(repeating: 1, count: 9))
        XCTAssertEqual(try Data(contentsOf: input), original)
    }

    func testHEICUsesFullResolutionJPEGEmbeddingInsteadOfInflatingEveryPhoto() throws {
        let dir = try directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let context = try XCTUnwrap(CGContext(data: nil, width: 160, height: 80, bitsPerComponent: 8, bytesPerRow: 640,
                                            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
        context.setFillColor(CGColor(red: 0.2, green: 0.6, blue: 0.8, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 160, height: 80))
        let input = dir.appendingPathComponent("phone.heic")
        let writer = try XCTUnwrap(CGImageDestinationCreateWithURL(input as CFURL, "public.heic" as CFString, 1, nil))
        CGImageDestinationAddImage(writer, try XCTUnwrap(context.makeImage()), [kCGImagePropertyOrientation: 6] as CFDictionary)
        XCTAssertTrue(CGImageDestinationFinalize(writer))
        let original = try Data(contentsOf: input)
        let output = dir.appendingPathComponent("converted.pdf")
        XCTAssertEqual(try ImagePDFImport.convert([input], to: output, layout: .init(size: nil, margin: 0)), 1)
        let data = try Data(contentsOf: output)
        XCTAssertNotNil(data.range(of: Data("/DCTDecode".utf8)))
        let page = try XCTUnwrap(PDFDocument(data: data)?.page(at: 0))
        XCTAssertEqual(page.bounds(for: .mediaBox).width, 80)
        XCTAssertEqual(page.bounds(for: .mediaBox).height, 160)
        XCTAssertEqual(try Data(contentsOf: input), original)
    }

    @MainActor func testNewMergeUsesSameAdmissionForEveryPositionAndHasNoPlaceholderUndo() throws {
        let doc = PDFDocument(); let page = PDFPage()
        page.setBounds(CGRect(x: 0, y: 0, width: 300, height: 400), for: .mediaBox)
        let link = PDFAnnotation(bounds: CGRect(x: 10, y: 10, width: 80, height: 20), forType: .link, withProperties: nil)
        link.url = URL(string: "https://example.test/reference"); page.addAnnotation(link)
        doc.insert(page, at: 0)
        let linked = try XCTUnwrap(doc.dataRepresentation())
        let plain = PDFWorkspace(); try plain.createBlank()
        let blank = try XCTUnwrap(plain.document?.dataRepresentation())
        for batch in [[linked, blank], [blank, linked]] {
            let workspace = PDFWorkspace(); try workspace.createUnsavedFileMerge(batch)
            XCTAssertEqual(workspace.document?.pageCount, 2)
            XCTAssertNil(workspace.sourceURL); XCTAssertTrue(workspace.isDirty)
            XCTAssertFalse(workspace.canUndo)
            XCTAssertEqual((0..<2).flatMap { workspace.document!.page(at: $0)!.annotations }.filter { $0.type == "Link" }.count, 1)
            let identity = workspace.document
            XCTAssertThrowsError(try workspace.createUnsavedFileMerge([blank, Data("bad".utf8)]))
            XCTAssertTrue(workspace.document === identity)
        }
    }
    func testMixedPDFImageAndTextBecomeOrderedIndependentPDFs() throws {
        let dir = try directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let base = dir.appendingPathComponent("base.pdf")
        let doc = PDFDocument(); let page = PDFPage(); page.setBounds(CGRect(x: 0, y: 0, width: 300, height: 400), for: .mediaBox)
        doc.insert(page, at: 0); XCTAssertTrue(doc.write(to: base))
        let png = try image(dir)
        let txt = dir.appendingPathComponent("notes.txt"); try Data("中文资料 Mixed files".utf8).write(to: txt)
        let originals = try [base, png, txt].map { try Data(contentsOf: $0) }
        let results = try FilePDFImport.prepare([base, png, txt])
        XCTAssertEqual(results.count, 3)
        guard results.count == 3 else { return }
        XCTAssertEqual(results[0], originals[0])
        XCTAssertEqual(results.compactMap { PDFDocument(data: $0)?.pageCount }, [1, 1, 1])
        XCTAssertTrue(PDFDocument(data: results[2])?.string?.contains("中文资料") == true)
        XCTAssertEqual(PDFDocument(data: results[1])?.page(at: 0)?.bounds(for: .mediaBox).size, CGSize(width: 80, height: 40))
        XCTAssertEqual(try [base, png, txt].map { try Data(contentsOf: $0) }, originals)
    }

    func testUnsupportedCorruptSymlinkDirectoryRemoteAndCancelledImportsFail() throws {
        let dir = try directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let png = try image(dir)
        let unsupported = dir.appendingPathComponent("video.mp4"); try Data([1, 2, 3]).write(to: unsupported)
        let corrupt = dir.appendingPathComponent("bad.png"); try Data("not a picture".utf8).write(to: corrupt)
        let link = dir.appendingPathComponent("link.png"); try FileManager.default.createSymbolicLink(at: link, withDestinationURL: png)
        for item in [unsupported, corrupt, link, dir, URL(string: "https://example.test/test.png")!] {
            XCTAssertThrowsError(try FilePDFImport.prepare([png, item]))
        }
        XCTAssertThrowsError(try FilePDFImport.prepare([png], cancelled: { true }))
        XCTAssertThrowsError(try FilePDFImport.prepare([]))
    }

    private func directory() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("FileImportTests-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: false); return url
    }
    private func image(_ dir: URL, name: String = "blue.png", width: Int = 80, height: Int = 40) throws -> URL {
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        for x in 0..<width { for y in 0..<height { bitmap.setColor(NSColor(deviceRed: 0.1, green: 0.3, blue: 0.8, alpha: 1), atX: x, y: y) } }
        let url = dir.appendingPathComponent(name)
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: url); return url
    }
}
