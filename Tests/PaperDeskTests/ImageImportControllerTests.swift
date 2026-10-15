import AppKit
import CoreGraphics
import ImageIO
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor
final class ImageImportControllerTests: XCTestCase {
    func testPaperOrientationAndMarginProduceActualEngineLayout() throws {
        let controller = ImageImportController()
        try controller.setLayout(paper: .a4, orientation: .landscape, marginText: "36")
        XCTAssertEqual(controller.layout.size?.width ?? 0, 841.89, accuracy: 0.001)
        XCTAssertEqual(controller.layout.size?.height ?? 0, 595.276, accuracy: 0.001)
        XCTAssertEqual(controller.layout.margin, 36)
        try controller.setLayout(paper: .letter, orientation: .portrait, marginText: "18.5")
        XCTAssertEqual(controller.layout.size, CGSize(width: 612, height: 792))
        XCTAssertEqual(controller.layout.margin, 18.5)
        try controller.setLayout(paper: .pixels, orientation: .landscape, marginText: "ignored")
        XCTAssertNil(controller.layout.size)
        XCTAssertEqual(controller.layout.margin, 0)
    }

    func testInvalidMarginsLeavePreviousConfigurationUnchanged() throws {
        let controller = ImageImportController()
        try controller.setLayout(paper: .letter, orientation: .portrait, marginText: "24")
        for invalid in ["", "abc", "nan", "inf", "-1", "306", "99999", "12 pt"] {
            XCTAssertThrowsError(try controller.setLayout(paper: .a4, orientation: .landscape, marginText: invalid))
            XCTAssertEqual(controller.paper, .letter)
            XCTAssertEqual(controller.layout.size, CGSize(width: 612, height: 792))
            XCTAssertEqual(controller.layout.margin, 24)
        }
    }

    func testInputOrderMoveRemovalAndDeduplicationAreAtomic() throws {
        let controller = ImageImportController()
        let files = [URL(fileURLWithPath: "/tmp/first.png"), URL(fileURLWithPath: "/tmp/second.JPG"), URL(fileURLWithPath: "/tmp/third.tiff")]
        try controller.replaceInputs(files + [files[0]])
        XCTAssertEqual(controller.inputs, files)
        XCTAssertTrue(controller.moveInput(at: 2, offset: -1))
        XCTAssertEqual(controller.inputs, [files[0], files[2], files[1]])
        XCTAssertFalse(controller.moveInput(at: 0, offset: -1))
        XCTAssertTrue(controller.removeInput(at: 1))
        XCTAssertEqual(controller.inputs, [files[0], files[1]])
        XCTAssertFalse(controller.removeInput(at: 100))
        XCTAssertThrowsError(try controller.replaceInputs([URL(string: "https://example.com/photo.png")!]))
        XCTAssertEqual(controller.inputs, [files[0], files[1]])
    }

    func testUnsupportedFormatsAndOversizedSelectionAreRejectedBeforeStarting() throws {
        let controller = ImageImportController()
        for suffix in ["pdf", "gif", "svg", "heic", "bmp"] {
            XCTAssertThrowsError(try controller.replaceInputs([URL(fileURLWithPath: "/tmp/file.\(suffix)")]))
        }
        XCTAssertThrowsError(try controller.replaceInputs((0..<501).map { URL(fileURLWithPath: "/tmp/image\($0).png") }))
        XCTAssertTrue(controller.inputs.isEmpty)
        XCTAssertFalse(controller.isRunning)
    }

    func testNativeConfigurationReflectsPixelModeAndChineseControls() throws {
        let controller = ImageImportController()
        _ = controller.view
        XCTAssertEqual(controller.paperChooser.accessibilityLabel(), "纸张大小")
        XCTAssertEqual(controller.orientationChooser.accessibilityLabel(), "页面方向")
        XCTAssertEqual(controller.marginField.accessibilityLabel(), "页边距，单位点")
        XCTAssertEqual(controller.fileTable.accessibilityLabel(), "图片顺序，每个文件按列表次序转换")
        XCTAssertFalse(controller.convertButton.isEnabled)
        try controller.replaceInputs([URL(fileURLWithPath: "/tmp/image.png")])
        try controller.setLayout(paper: .pixels, orientation: .portrait, marginText: "0")
        XCTAssertFalse(controller.orientationChooser.isEnabled)
        XCTAssertFalse(controller.marginField.isEnabled)
        XCTAssertTrue(controller.convertButton.isEnabled)
        XCTAssertTrue(controller.formatNotice.stringValue.contains("TIFF"))
        XCTAssertTrue(controller.formatNotice.stringValue.contains("1 像素"))
    }

    func testNativeSheetContentFitsItsBoundsAndOrderButtonsUseTheModel() throws {
        let controller = ImageImportController()
        let root = controller.view
        let files = [URL(fileURLWithPath: "/tmp/first.png"), URL(fileURLWithPath: "/tmp/second.jpeg")]
        try controller.replaceInputs(files)
        controller.fileTable.selectRowIndexes(IndexSet(integer: 1), byExtendingSelection: false)
        root.layoutSubtreeIfNeeded()
        let up = try XCTUnwrap(findControl(in: root, label: "上移图片"))
        XCTAssertTrue(up.sendAction(up.action, to: up.target))
        XCTAssertEqual(controller.inputs, files.reversed())
        for label in ["纸张大小", "页面方向", "页边距，单位点", "上移图片", "下移图片", "移除图片，不删除来源文件", "取消图片导入", "创建新 PDF，不覆盖当前文档"] {
            let control = try XCTUnwrap(findControl(in: root, label: label))
            let frame = root.convert(control.bounds, from: control)
            XCTAssertGreaterThan(frame.width, 0, label)
            XCTAssertGreaterThanOrEqual(frame.minX, 0, label)
            XCTAssertLessThanOrEqual(frame.maxX, root.bounds.width, label)
            XCTAssertGreaterThanOrEqual(frame.minY, 0, label)
            XCTAssertLessThanOrEqual(frame.maxY, root.bounds.height, label)
        }
    }

    private func findControl(in view: NSView, label: String) -> NSControl? {
        if view.accessibilityLabel() == label, let control = view as? NSControl { return control }
        return view.subviews.lazy.compactMap { self.findControl(in: $0, label: label) }.first
    }

    func testRealConversionPreservesSourcesAndEmitsNewDocumentDataInSelectedOrder() async throws {
        let fixture = try makeImages()
        defer { try? FileManager.default.removeItem(at: fixture.directory) }
        let controller = ImageImportController()
        try controller.replaceInputs(fixture.files)
        XCTAssertTrue(controller.moveInput(at: 1, offset: -1))
        try controller.setLayout(paper: .pixels, orientation: .portrait, marginText: "0")
        var published: [ImageImportOutput] = []
        controller.onImport = { published.append($0) }
        let result = try await controller.convert()
        let document = try XCTUnwrap(PDFDocument(data: result.data))
        XCTAssertEqual(document.pageCount, 2)
        XCTAssertEqual(result.pageCount, 2)
        XCTAssertEqual(document.page(at: 0)?.bounds(for: .mediaBox).size, CGSize(width: 20, height: 40))
        XCTAssertEqual(document.page(at: 1)?.bounds(for: .mediaBox).size, CGSize(width: 40, height: 20))
        XCTAssertEqual(published.count, 1)
        XCTAssertEqual(published.first?.data, result.data)
        XCTAssertTrue(result.suggestedName.hasSuffix(".pdf"))
        XCTAssertFalse(controller.isRunning)
        for (index, url) in fixture.files.enumerated() { XCTAssertEqual(try Data(contentsOf: url), fixture.originals[index]) }
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: fixture.directory.path).count, 2)
    }

    func testLetterLandscapeConversionUsesConfiguredGeometry() async throws {
        let fixture = try makeImages()
        defer { try? FileManager.default.removeItem(at: fixture.directory) }
        let controller = ImageImportController()
        try controller.replaceInputs([fixture.files[0]])
        try controller.setLayout(paper: .letter, orientation: .landscape, marginText: "12")
        let result = try await controller.convert()
        let doc = try XCTUnwrap(PDFDocument(data: result.data))
        XCTAssertEqual(doc.page(at: 0)?.bounds(for: .mediaBox).size, CGSize(width: 792, height: 612))
    }

    func testJPEGAndEveryTIFFFrameBecomePagesInListOrder() async throws {
        let fixture = try makeImages()
        defer { try? FileManager.default.removeItem(at: fixture.directory) }
        let first = try XCTUnwrap(CGImageSourceCreateWithURL(fixture.files[0] as CFURL, nil))
        let second = try XCTUnwrap(CGImageSourceCreateWithURL(fixture.files[1] as CFURL, nil))
        let imageA = try XCTUnwrap(CGImageSourceCreateImageAtIndex(first, 0, nil))
        let imageB = try XCTUnwrap(CGImageSourceCreateImageAtIndex(second, 0, nil))
        let jpeg = fixture.directory.appendingPathComponent("first.jpeg")
        let jpegWriter = try XCTUnwrap(CGImageDestinationCreateWithURL(jpeg as CFURL, "public.jpeg" as CFString, 1, nil))
        CGImageDestinationAddImage(jpegWriter, imageA, nil)
        XCTAssertTrue(CGImageDestinationFinalize(jpegWriter))
        let tiff = fixture.directory.appendingPathComponent("two frames.tiff")
        let tiffWriter = try XCTUnwrap(CGImageDestinationCreateWithURL(tiff as CFURL, "public.tiff" as CFString, 2, nil))
        CGImageDestinationAddImage(tiffWriter, imageA, nil)
        CGImageDestinationAddImage(tiffWriter, imageB, nil)
        XCTAssertTrue(CGImageDestinationFinalize(tiffWriter))
        let originals = try [jpeg, tiff].map { try Data(contentsOf: $0) }
        let controller = ImageImportController()
        try controller.replaceInputs([jpeg, tiff])
        try controller.setLayout(paper: .pixels, orientation: .portrait, marginText: "0")
        let result = try await controller.convert()
        let document = try XCTUnwrap(PDFDocument(data: result.data))
        XCTAssertEqual(result.pageCount, 3)
        XCTAssertEqual(document.pageCount, 3)
        XCTAssertEqual(document.page(at: 0)?.bounds(for: .mediaBox).size, CGSize(width: 40, height: 20))
        XCTAssertEqual(document.page(at: 1)?.bounds(for: .mediaBox).size, CGSize(width: 40, height: 20))
        XCTAssertEqual(document.page(at: 2)?.bounds(for: .mediaBox).size, CGSize(width: 20, height: 40))
        XCTAssertEqual(try Data(contentsOf: jpeg), originals[0])
        XCTAssertEqual(try Data(contentsOf: tiff), originals[1])
    }

    func testCancelBeforeBackgroundWorkSuppressesPublicationAndKeepsSources() async throws {
        let fixture = try makeImages()
        defer { try? FileManager.default.removeItem(at: fixture.directory) }
        let controller = ImageImportController()
        try controller.replaceInputs(fixture.files)
        var publications = 0
        controller.onImport = { _ in publications += 1 }
        controller.onStatus = { _ in if controller.isRunning { controller.cancel() } }
        do { _ = try await controller.convert(); XCTFail("Cancelled conversion must not publish") }
        catch { XCTAssertEqual(error as? ImageImportControllerFailure, .cancelled) }
        XCTAssertEqual(publications, 0)
        XCTAssertFalse(controller.isRunning)
        XCTAssertEqual(try Data(contentsOf: fixture.files[0]), fixture.originals[0])
    }

    func testSessionInvalidationSuppressesStaleConversionCallback() async throws {
        let fixture = try makeImages()
        defer { try? FileManager.default.removeItem(at: fixture.directory) }
        let controller = ImageImportController()
        try controller.replaceInputs(fixture.files)
        var publications = 0
        controller.onImport = { _ in publications += 1 }
        controller.onStatus = { _ in if controller.isRunning { controller.invalidate() } }
        do { _ = try await controller.convert(); XCTFail("Stale conversion must not publish") }
        catch { XCTAssertEqual(error as? ImageImportControllerFailure, .staleResult) }
        XCTAssertEqual(publications, 0)
        XCTAssertFalse(controller.isRunning)
    }

    func testEmptyOrBrokenInputNeverEmitsDocumentData() async throws {
        let controller = ImageImportController()
        var publications = 0
        controller.onImport = { _ in publications += 1 }
        do { _ = try await controller.convert(); XCTFail("Empty selection") }
        catch { XCTAssertEqual(error as? ImageImportControllerFailure, .invalidInput) }
        try controller.replaceInputs([URL(fileURLWithPath: "/tmp/PDFMaster-absent-\(UUID().uuidString).png")])
        do { _ = try await controller.convert(); XCTFail("Missing image") } catch {}
        XCTAssertEqual(publications, 0)
        XCTAssertFalse(controller.isRunning)
    }

    func testNativeStartThenCancelInSameTurnNeverPublishesOrReportsFailure() async throws {
        try await assertQueuedStartIsDiscarded { controller in
            let cancel = try XCTUnwrap(self.findControl(in: controller.view, label: "取消图片导入"))
            XCTAssertTrue(cancel.sendAction(cancel.action, to: cancel.target))
        }
    }

    func testNativeStartThenParentInvalidateInSameTurnNeverPublishes() async throws {
        try await assertQueuedStartIsDiscarded { $0.invalidate() }
    }

    func testNativeStartThenWindowCloseInSameTurnNeverPublishes() async throws {
        try await assertQueuedStartIsDiscarded { controller in
            XCTAssertFalse(controller.windowShouldClose(NSWindow()))
        }
    }

    func testNativeDoubleStartReservesOneJobBeforeYielding() async throws {
        let fixture = try makeImages()
        defer { try? FileManager.default.removeItem(at: fixture.directory) }
        let controller = ImageImportController()
        _ = controller.view
        try controller.replaceInputs(fixture.files)
        let published = expectation(description: "one document")
        var publicationCount = 0, failures = 0, statuses = 0
        controller.onImport = { _ in publicationCount += 1; published.fulfill() }
        controller.onFailure = { _ in failures += 1 }
        controller.onStatus = { _ in statuses += 1 }
        XCTAssertTrue(controller.convertButton.sendAction(controller.convertButton.action, to: controller.convertButton.target))
        XCTAssertTrue(controller.isRunning, "The button action must reserve the job synchronously")
        XCTAssertFalse(controller.convertButton.isEnabled)
        XCTAssertTrue(controller.convertButton.sendAction(controller.convertButton.action, to: controller.convertButton.target))
        await fulfillment(of: [published], timeout: 5)
        XCTAssertEqual(publicationCount, 1)
        XCTAssertEqual(failures, 0)
        XCTAssertEqual(statuses, 1)
        XCTAssertFalse(controller.isRunning)
    }

    func testNativeStatusCallbackCanReenterCancelWithoutTerminalCallbacks() async throws {
        let fixture = try makeImages()
        defer { try? FileManager.default.removeItem(at: fixture.directory) }
        let controller = ImageImportController()
        _ = controller.view
        try controller.replaceInputs(fixture.files)
        let unwanted = expectation(description: "cancelled UI job has no terminal callback")
        unwanted.isInverted = true
        let status = expectation(description: "status delivered")
        controller.onImport = { _ in unwanted.fulfill() }
        controller.onFailure = { _ in unwanted.fulfill() }
        controller.onStatus = { _ in controller.cancel(); status.fulfill() }
        XCTAssertTrue(controller.convertButton.sendAction(controller.convertButton.action, to: controller.convertButton.target))
        await fulfillment(of: [status], timeout: 5)
        await fulfillment(of: [unwanted], timeout: 0.3)
        XCTAssertFalse(controller.isRunning)
        XCTAssertEqual(try Data(contentsOf: fixture.files[0]), fixture.originals[0])
    }

    private func assertQueuedStartIsDiscarded(_ discard: (ImageImportController) throws -> Void) async throws {
        let fixture = try makeImages()
        defer { try? FileManager.default.removeItem(at: fixture.directory) }
        let controller = ImageImportController()
        _ = controller.view
        try controller.replaceInputs(fixture.files)
        let unwanted = expectation(description: "discarded queued start has no callbacks")
        unwanted.isInverted = true
        var statuses = 0
        controller.onImport = { _ in unwanted.fulfill() }
        controller.onFailure = { _ in unwanted.fulfill() }
        controller.onStatus = { _ in statuses += 1 }
        XCTAssertTrue(controller.convertButton.sendAction(controller.convertButton.action, to: controller.convertButton.target))
        XCTAssertTrue(controller.isRunning, "The start action must own a cancellation flag before yielding")
        try discard(controller) // Intentionally no await between the two actions.
        await fulfillment(of: [unwanted], timeout: 0.3)
        XCTAssertEqual(statuses, 0, "Discarded queued tasks must not enter conversion")
        XCTAssertFalse(controller.isRunning)
        for (index, file) in fixture.files.enumerated() {
            XCTAssertEqual(try Data(contentsOf: file), fixture.originals[index])
        }
    }

    func testRenamedGIFIsRejectedByItsActualEncodingNotOnlyItsSuffix() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-import-format-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: directory) }
        let input = directory.appendingPathComponent("misnamed.png")
        let gif = try XCTUnwrap(Data(base64Encoded: "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7"))
        try gif.write(to: input)
        let controller = ImageImportController()
        try controller.replaceInputs([input])
        var publications = 0
        controller.onImport = { _ in publications += 1 }
        do { _ = try await controller.convert(); XCTFail("GIF is outside the advertised subset") }
        catch { XCTAssertEqual(error as? ImageImportControllerFailure, .unsupportedFormat) }
        XCTAssertEqual(publications, 0)
        XCTAssertEqual(try Data(contentsOf: input), gif)
    }

    private func makeImages() throws -> (directory: URL, files: [URL], originals: [Data]) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-import-controller-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        var files: [URL] = []
        for (name, width, height, color) in [("red landscape.png", 40, 20, NSColor.red), ("blue portrait.png", 20, 40, NSColor.blue)] {
            let context = try XCTUnwrap(CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
            context.setFillColor(color.cgColor)
            context.fill(CGRect(x: 0, y: 0, width: width, height: height))
            let image = try XCTUnwrap(context.makeImage())
            let url = directory.appendingPathComponent(name)
            let destination = try XCTUnwrap(CGImageDestinationCreateWithURL(url as CFURL, "public.png" as CFString, 1, nil))
            CGImageDestinationAddImage(destination, image, nil)
            XCTAssertTrue(CGImageDestinationFinalize(destination))
            files.append(url)
        }
        return (directory, files, try files.map { try Data(contentsOf: $0) })
    }
}
