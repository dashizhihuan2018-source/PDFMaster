import AppKit
import ObjectiveC.runtime
import PDFKit
import XCTest
@testable import PaperCore

private enum PDFSerializationCounter {
    private static let lock = NSLock()
    private static var storage = 0

    static var value: Int {
        lock.lock()
        defer { lock.unlock() }
        return storage
    }

    static func reset() {
        lock.lock()
        storage = 0
        lock.unlock()
    }

    static func increment() {
        lock.lock()
        storage += 1
        lock.unlock()
    }
}

private enum PDFPageCopyCounter {
    private static let lock = NSLock()
    private static var storage = 0

    static var value: Int {
        lock.lock()
        defer { lock.unlock() }
        return storage
    }

    static func reset() {
        lock.lock()
        storage = 0
        lock.unlock()
    }

    static func increment() {
        lock.lock()
        storage += 1
        lock.unlock()
    }
}

private enum PDFDataInitializationFailure {
    private static let lock = NSLock()
    private static var remainingFailures = 0

    static func failNext() {
        lock.lock()
        remainingFailures = 1
        lock.unlock()
    }

    static func consume() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard remainingFailures > 0 else { return false }
        remainingFailures -= 1
        return true
    }
}

private extension PDFDocument {
    @objc dynamic func paperCorePerformanceDataRepresentation() -> Data? {
        PDFSerializationCounter.increment()
        // Swizzling makes this selector point at PDFKit's original implementation.
        return paperCorePerformanceDataRepresentation()
    }
}

private extension PDFPage {
    @objc dynamic func paperCorePerformanceCopy(with zone: NSZone?) -> Any {
        PDFPageCopyCounter.increment()
        // Swizzling makes this selector point at PDFKit's original implementation.
        return paperCorePerformanceCopy(with: zone)
    }
}

@MainActor
final class PageOperationPerformanceTests: XCTestCase {
    private var temporaryDirectory: URL!

    override func setUpWithError() throws {
        temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("PageOperationPerformanceTests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: temporaryDirectory, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        if temporaryDirectory != nil {
            try? FileManager.default.removeItem(at: temporaryDirectory)
        }
    }

    func test269PageInsertAndDeleteUseOneFullSerializationEach() throws {
        let source = try makePDF(named: "synthetic-269-pages.pdf", pageCount: 269)
        let sourceBytes = try Data(contentsOf: source)
        let workspace = PDFWorkspace()
        try workspace.open(source)

        let insert = try countSerializations {
            try workspace.insertBlank(at: 134)
        }
        XCTAssertEqual(workspace.document?.pageCount, 270)
        XCTAssertEqual(insert.calls, 1, "Insertion must not serialize the complete document more than once")

        let incoming = try makePDF(named: "synthetic-incoming-page.pdf", pageCount: 1)
        let insertPDF = try countSerializations {
            try workspace.insertPDF(incoming, at: 135)
        }
        XCTAssertEqual(workspace.document?.pageCount, 271)
        XCTAssertEqual(insertPDF.calls, 1, "PDF insertion must not serialize the complete document more than once")

        let delete = try countSerializations {
            try workspace.delete(pages: IndexSet([134, 135]))
        }
        XCTAssertEqual(workspace.document?.pageCount, 269)
        XCTAssertEqual(delete.calls, 1, "Deletion must not serialize the complete document more than once")
        XCTAssertEqual(try Data(contentsOf: source), sourceBytes, "In-memory page operations must not alter source bytes")

        print(
            String(
                format: "PAGE_OPERATION_PERF pages=269 insert_seconds=%.6f insert_serializations=%d insert_pdf_seconds=%.6f insert_pdf_serializations=%d delete_seconds=%.6f delete_serializations=%d",
                insert.seconds,
                insert.calls,
                insertPDF.seconds,
                insertPDF.calls,
                delete.seconds,
                delete.calls
            )
        )
    }

    func testCommentTransactionsAvoidSecondWholePDFRoundTripAndStillUndo() throws {
        let source = try makePDF(named: "comment-269.pdf", pageCount: 269)
        let workspace = PDFWorkspace(); try workspace.open(source)
        let add = try countSerializations { try workspace.addShape(.rectangle, page: 134, bounds: CGRect(x: 40, y: 40, width: 70, height: 60), style: .default) }
        XCTAssertEqual(add.calls, 1)
        let erase = try countSerializations { try workspace.removeAnnotation(page: 134, index: 0) }
        XCTAssertEqual(erase.calls, 1)
        XCTAssertEqual(workspace.document?.page(at: 134)?.annotations.count, 0)
        workspace.undo()
        XCTAssertEqual(workspace.document?.page(at: 134)?.annotations.first?.type, "Square")
    }

    func testOptimizedPageMutationsPreserveAnnotationsUndoRedoAndSaveReopen() throws {
        let source = try makePDF(named: "annotated-pages.pdf", pageCount: 3, annotatePage: 1)
        let workspace = PDFWorkspace()
        try workspace.open(source)

        try workspace.insertBlank(at: 1)
        XCTAssertEqual(workspace.document?.page(at: 2)?.annotations.first?.contents, "keep-me")
        workspace.undo()
        XCTAssertEqual(workspace.document?.pageCount, 3)
        XCTAssertEqual(workspace.document?.page(at: 1)?.annotations.first?.contents, "keep-me")
        workspace.redo()
        XCTAssertEqual(workspace.document?.pageCount, 4)
        XCTAssertEqual(workspace.document?.page(at: 2)?.annotations.first?.contents, "keep-me")

        try workspace.delete(pages: IndexSet(integer: 0))
        let output = temporaryDirectory.appendingPathComponent("round-trip.pdf")
        try workspace.saveCopy(to: output)
        let reopened = try XCTUnwrap(PDFDocument(url: output))
        XCTAssertEqual(reopened.pageCount, 3)
        XCTAssertEqual(reopened.page(at: 1)?.annotations.first?.contents, "keep-me")
    }

    func testInsertPDFRejectsEmptyUserPasswordEncryptionWithoutChangingWorkspace() throws {
        let encrypted = try makeEmptyUserPasswordEncryptedPDF(named: "encrypted-incoming.pdf")
        let raw = try Data(contentsOf: encrypted)
        let admittedByPDFKit = try XCTUnwrap(PDFDocument(data: raw))
        XCTAssertTrue(admittedByPDFKit.isEncrypted)
        XCTAssertFalse(admittedByPDFKit.isLocked)

        let workspace = PDFWorkspace()
        try workspace.createBlank()
        let beforeDocument = workspace.document
        let beforeRevision = workspace.revision
        let beforeDirty = workspace.isDirty

        XCTAssertThrowsError(try workspace.insertPDF(encrypted, at: 1)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied)
        }
        XCTAssertTrue(workspace.document === beforeDocument)
        XCTAssertEqual(workspace.document?.pageCount, 1)
        XCTAssertEqual(workspace.revision, beforeRevision)
        XCTAssertEqual(workspace.isDirty, beforeDirty)
        XCTAssertFalse(workspace.canUndo)
    }

    func testInsertPDFRejectsInvalidIndexBeforeCopyingIncomingPages() throws {
        let incoming = try makePDF(named: "preflight-incoming.pdf", pageCount: 3)
        let workspace = PDFWorkspace()
        try workspace.createBlank()

        let copyCalls = try countPageCopies {
            XCTAssertThrowsError(try workspace.insertPDF(incoming, at: 2)) {
                XCTAssertEqual($0 as? PDFWorkspaceError, .invalidPage)
            }
        }

        XCTAssertEqual(copyCalls, 0, "Invalid index must fail before copying any incoming page")
        XCTAssertEqual(workspace.document?.pageCount, 1)
        XCTAssertEqual(workspace.revision, 0)
        XCTAssertFalse(workspace.canUndo)
    }

    func testInsertPDFRejectsCombinedPageLimitBeforeCopyingIncomingPages() throws {
        let incoming = try makePDF(named: "page-limit-incoming.pdf", pageCount: 2)
        let workspace = PDFWorkspace(historyByteLimit: 256 * 1024 * 1024, pageLimit: 2)
        try workspace.createBlank()

        let copyCalls = try countPageCopies {
            XCTAssertThrowsError(try workspace.insertPDF(incoming, at: 1)) {
                XCTAssertEqual($0 as? PDFWorkspaceError, .invalidPage)
            }
        }

        XCTAssertEqual(copyCalls, 0, "Combined page limit must fail before copying any incoming page")
        XCTAssertEqual(workspace.document?.pageCount, 1)
        XCTAssertEqual(workspace.revision, 0)
        XCTAssertFalse(workspace.canUndo)
    }

    func testUndoAndRedoKeepHistoryWhenSnapshotRestorationFails() throws {
        let workspace = PDFWorkspace()
        try workspace.createBlank()
        try workspace.insertBlank(at: 1)

        withNextDataInitializationFailure {
            workspace.undo()
        }
        XCTAssertEqual(workspace.document?.pageCount, 2)
        XCTAssertTrue(workspace.canUndo, "Failed undo restoration must not consume its snapshot")
        XCTAssertFalse(workspace.canRedo)

        workspace.undo()
        XCTAssertEqual(workspace.document?.pageCount, 1)
        XCTAssertFalse(workspace.canUndo)
        XCTAssertTrue(workspace.canRedo)

        withNextDataInitializationFailure {
            workspace.redo()
        }
        XCTAssertEqual(workspace.document?.pageCount, 1)
        XCTAssertTrue(workspace.canRedo, "Failed redo restoration must not consume its snapshot")

        workspace.redo()
        XCTAssertEqual(workspace.document?.pageCount, 2)
        XCTAssertTrue(workspace.canUndo)
        XCTAssertFalse(workspace.canRedo)
    }

    func testHistoryByteBudgetEvictsOldestSnapshotAndPreservesNearestDirtyUndo() throws {
        let sizingWorkspace = PDFWorkspace()
        try sizingWorkspace.createBlank()
        let onePageBytes = try XCTUnwrap(sizingWorkspace.document?.dataRepresentation()).count
        try sizingWorkspace.insertBlank(at: 1)
        let twoPageBytes = try XCTUnwrap(sizingWorkspace.document?.dataRepresentation()).count
        let budgetForExactlyOneOfTheTwoSnapshots = onePageBytes + twoPageBytes - 1

        let workspace = PDFWorkspace(historyByteLimit: budgetForExactlyOneOfTheTwoSnapshots)
        try workspace.createBlank()
        try workspace.insertBlank(at: 1)
        try workspace.insertBlank(at: 2)
        XCTAssertEqual(workspace.document?.pageCount, 3)
        XCTAssertTrue(workspace.canUndo)

        workspace.undo()
        XCTAssertEqual(workspace.document?.pageCount, 2)
        XCTAssertTrue(workspace.isDirty, "The retained nearest snapshot was already dirty")
        XCTAssertFalse(workspace.canUndo, "The oldest clean snapshot must be evicted by the byte budget")
        XCTAssertTrue(workspace.canRedo)

        workspace.redo()
        XCTAssertEqual(workspace.document?.pageCount, 3)
        XCTAssertTrue(workspace.isDirty)
        XCTAssertTrue(workspace.canUndo)
        XCTAssertFalse(workspace.canRedo)
    }

    private func countSerializations(_ operation: () throws -> Void) rethrows -> (calls: Int, seconds: TimeInterval) {
        let originalSelector = NSSelectorFromString("dataRepresentation")
        let countedSelector = #selector(PDFDocument.paperCorePerformanceDataRepresentation)
        guard let original = class_getInstanceMethod(PDFDocument.self, originalSelector),
              let counted = class_getInstanceMethod(PDFDocument.self, countedSelector) else {
            XCTFail("PDFKit dataRepresentation instrumentation is unavailable")
            try operation()
            return (0, 0)
        }

        method_exchangeImplementations(original, counted)
        PDFSerializationCounter.reset()
        let start = ProcessInfo.processInfo.systemUptime
        defer { method_exchangeImplementations(counted, original) }
        try operation()
        return (PDFSerializationCounter.value, ProcessInfo.processInfo.systemUptime - start)
    }

    private func countPageCopies(_ operation: () throws -> Void) rethrows -> Int {
        let originalSelector = NSSelectorFromString("copyWithZone:")
        let countedSelector = #selector(PDFPage.paperCorePerformanceCopy(with:))
        guard let original = class_getInstanceMethod(PDFPage.self, originalSelector),
              let counted = class_getInstanceMethod(PDFPage.self, countedSelector) else {
            XCTFail("PDFKit PDFPage.copy instrumentation is unavailable")
            try operation()
            return 0
        }

        method_exchangeImplementations(original, counted)
        PDFPageCopyCounter.reset()
        defer { method_exchangeImplementations(counted, original) }
        try operation()
        return PDFPageCopyCounter.value
    }

    private func withNextDataInitializationFailure(_ operation: () -> Void) {
        let selector = NSSelectorFromString("initWithData:")
        guard let method = class_getInstanceMethod(PDFDocument.self, selector) else {
            XCTFail("PDFKit init(data:) instrumentation is unavailable")
            operation()
            return
        }

        typealias OriginalInitializer = @convention(c) (AnyObject, Selector, NSData) -> AnyObject?
        let originalImplementation = method_getImplementation(method)
        let original = unsafeBitCast(originalImplementation, to: OriginalInitializer.self)
        let block: @convention(block) (AnyObject, NSData) -> AnyObject? = { object, data in
            guard !PDFDataInitializationFailure.consume() else { return nil }
            return original(object, selector, data)
        }
        let replacement = imp_implementationWithBlock(block)
        method_setImplementation(method, replacement)
        PDFDataInitializationFailure.failNext()
        defer {
            method_setImplementation(method, originalImplementation)
            imp_removeBlock(replacement)
        }
        operation()
    }

    private func makePDF(named name: String, pageCount: Int, annotatePage: Int? = nil) throws -> URL {
        let url = temporaryDirectory.appendingPathComponent(name)
        guard let consumer = CGDataConsumer(url: url as CFURL) else { throw FixtureError.writeFailed }
        var mediaBox = CGRect(x: 0, y: 0, width: 612, height: 792)
        guard let context = CGContext(consumer: consumer, mediaBox: &mediaBox, nil) else {
            throw FixtureError.writeFailed
        }
        for page in 0..<pageCount {
            context.beginPDFPage(nil)
            context.saveGState()
            let graphicsContext = NSGraphicsContext(cgContext: context, flipped: false)
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = graphicsContext
            ("Synthetic performance page \(page + 1)" as NSString).draw(
                at: NSPoint(x: 72, y: 700),
                withAttributes: [.font: NSFont.systemFont(ofSize: 18), .foregroundColor: NSColor.black]
            )
            NSGraphicsContext.restoreGraphicsState()
            context.restoreGState()
            context.endPDFPage()
        }
        context.closePDF()

        if let annotatePage {
            let document = try XCTUnwrap(PDFDocument(url: url))
            let page = try XCTUnwrap(document.page(at: annotatePage))
            let annotation = PDFAnnotation(
                bounds: CGRect(x: 72, y: 620, width: 140, height: 30),
                forType: .freeText,
                withProperties: nil
            )
            annotation.contents = "keep-me"
            page.addAnnotation(annotation)
            XCTAssertTrue(document.write(to: url))
        }

        XCTAssertEqual(PDFDocument(url: url)?.pageCount, pageCount)
        return url
    }

    private func makeEmptyUserPasswordEncryptedPDF(named name: String) throws -> URL {
        let plain = try makePDF(named: "plain-\(name)", pageCount: 1)
        let document = try XCTUnwrap(PDFDocument(url: plain))
        let output = temporaryDirectory.appendingPathComponent(name)
        XCTAssertTrue(document.write(
            to: output,
            withOptions: [
                PDFDocumentWriteOption.userPasswordOption: "",
                PDFDocumentWriteOption.ownerPasswordOption: "owner-password",
                PDFDocumentWriteOption.accessPermissionsOption: NSNumber(value: UInt.max)
            ]
        ))
        return output
    }

    private enum FixtureError: Error { case writeFailed }
}
