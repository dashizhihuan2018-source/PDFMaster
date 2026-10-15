import AppKit
import PDFKit
import ObjectiveC.runtime
import XCTest
@testable import PaperCore

private enum ReorderSerializationCounter {
    private static let lock = NSLock()
    private static var count = 0
    static var value: Int { lock.lock(); defer { lock.unlock() }; return count }
    static func reset() { lock.lock(); count = 0; lock.unlock() }
    static func increment() { lock.lock(); count += 1; lock.unlock() }
}

private extension PDFDocument {
    @objc dynamic func paperReorderCountedDataRepresentation() -> Data? {
        ReorderSerializationCounter.increment()
        return paperReorderCountedDataRepresentation()
    }
}

@MainActor
final class PageReorderTests: XCTestCase {
    func testNoncontiguousMultiPageMoveToEndIsOneUndoAndRedo() throws {
        let (workspace, source) = try fixture()
        let originalBytes = try Data(contentsOf: source)
        try workspace.movePages(pages: [1, 3], to: 5)
        XCTAssertEqual(order(workspace), [0, 2, 4, 1, 3])
        XCTAssertEqual(workspace.revision, 1)
        XCTAssertTrue(workspace.isDirty)
        XCTAssertEqual(workspace.undo(), .document)
        XCTAssertEqual(order(workspace), Array(0..<5))
        XCTAssertFalse(workspace.canUndo)
        XCTAssertFalse(workspace.isDirty)
        XCTAssertEqual(workspace.redo(), .document)
        XCTAssertEqual(order(workspace), [0, 2, 4, 1, 3])
        XCTAssertFalse(workspace.canRedo)
        XCTAssertEqual(try Data(contentsOf: source), originalBytes)
    }

    func testMovesUseOriginalDocumentInsertionBoundaries() throws {
        for (pages, insertion, expected) in [
            ([1, 3], 0, [1, 3, 0, 2, 4]),
            ([1, 3], 2, [0, 1, 3, 2, 4]),
            ([1, 2], 4, [0, 3, 1, 2, 4]),
            ([3, 4], 1, [0, 3, 4, 1, 2]),
            ([0], 5, [1, 2, 3, 4, 0]),
            ([4], 0, [4, 0, 1, 2, 3])
        ] {
            let (workspace, _) = try fixture()
            try workspace.movePages(pages: IndexSet(pages), to: insertion)
            XCTAssertEqual(order(workspace), expected, "pages=\(pages), insertion=\(insertion)")
            XCTAssertEqual(workspace.document?.pageCount, 5)
        }
    }

    func testNoOpContiguousAllAndSinglePageMovesLeaveDocumentHistoryAndRevisionUntouched() throws {
        for (pages, boundaries, pageCount) in [
            ([1, 2], [1, 2, 3], 5),
            (Array(0..<5), Array(0...5), 5),
            ([0], [0, 1], 1)
        ] {
            let (workspace, _) = try fixture(pageCount: pageCount)
            let document = workspace.document
            for boundary in boundaries {
                try workspace.movePages(pages: IndexSet(pages), to: boundary)
                XCTAssertTrue(workspace.document === document)
                XCTAssertEqual(workspace.revision, 0)
                XCTAssertFalse(workspace.isDirty)
                XCTAssertFalse(workspace.canUndo)
                XCTAssertFalse(workspace.canRedo)
            }
        }
    }

    func testNoOpDoesNotClearExistingRedoHistory() throws {
        let (workspace, _) = try fixture()
        try workspace.movePages(pages: [0], to: 5)
        workspace.undo()
        let document = workspace.document
        let revision = workspace.revision
        try workspace.movePages(pages: [1, 2], to: 2)
        XCTAssertTrue(workspace.document === document)
        XCTAssertEqual(workspace.revision, revision)
        XCTAssertTrue(workspace.canRedo)
        workspace.redo()
        XCTAssertEqual(order(workspace), [1, 2, 3, 4, 0])
    }

    func testInvalidSelectionAndDestinationFailWithoutAnyPublishedMutation() throws {
        let (workspace, _) = try fixture()
        let document = workspace.document
        for (pages, insertion) in [(IndexSet(), 0), (IndexSet([0, 5]), 0), (IndexSet([0]), -1), (IndexSet([0]), 6)] {
            XCTAssertThrowsError(try workspace.movePages(pages: pages, to: insertion)) {
                XCTAssertEqual($0 as? PDFWorkspaceError, .invalidPage)
            }
            XCTAssertTrue(workspace.document === document)
            XCTAssertEqual(order(workspace), Array(0..<5))
            XCTAssertEqual(workspace.revision, 0)
            XCTAssertFalse(workspace.isDirty)
            XCTAssertFalse(workspace.canUndo)
        }
        XCTAssertThrowsError(try PDFWorkspace().movePages(pages: [0], to: 0)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .noDocument)
        }
    }

    func testAssemblyPermissionIsEnforcedForActualAndNoOpMoves() throws {
        let (workspace, _) = try fixture(restricted: true)
        XCTAssertFalse(workspace.allowsDocumentAssembly)
        let document = workspace.document
        for insertion in [1, 5] {
            XCTAssertThrowsError(try workspace.movePages(pages: [1], to: insertion)) {
                XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied)
            }
            XCTAssertTrue(workspace.document === document)
            XCTAssertEqual(workspace.revision, 0)
            XCTAssertFalse(workspace.canUndo)
        }
    }

    func testInvalidOutlineFailsWithoutConsumingRedoHistory() throws {
        let (workspace, _) = try fixture()
        try workspace.movePages(pages: [0], to: 5)
        workspace.undo()
        let document = try XCTUnwrap(workspace.document)
        let invalid = PDFOutline(); invalid.label = "Missing destination"
        document.outlineRoot?.insertChild(invalid, at: 1)
        let revision = workspace.revision
        XCTAssertThrowsError(try workspace.movePages(pages: [0], to: 5)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .invalidSelection)
        }
        XCTAssertTrue(workspace.document === document)
        XCTAssertEqual(workspace.revision, revision)
        XCTAssertFalse(workspace.isDirty)
        XCTAssertFalse(workspace.canUndo)
        XCTAssertTrue(workspace.canRedo)
        XCTAssertEqual(order(workspace), Array(0..<5))
    }

    func testCloneSnapshotFailureLeavesDocumentRevisionDirtyAndRedoUntouched() throws {
        let (workspace, _) = try fixture()
        try workspace.movePages(pages: [0], to: 5)
        workspace.undo()
        let document = workspace.document
        let revision = workspace.revision
        let method = try XCTUnwrap(class_getInstanceMethod(PDFDocument.self, #selector(PDFDocument.dataRepresentation)))
        let original = method_getImplementation(method)
        // Inject failure only at PDFKit's snapshot boundary, not the workspace
        // transaction under test. The original implementation is always restored.
        let block: @convention(block) (AnyObject) -> NSData? = { _ in nil }
        let failure = imp_implementationWithBlock(block)
        method_setImplementation(method, failure)
        defer { method_setImplementation(method, original); imp_removeBlock(failure) }
        XCTAssertThrowsError(try workspace.movePages(pages: [0], to: 5)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .unreadableDocument)
        }
        XCTAssertTrue(workspace.document === document)
        XCTAssertEqual(workspace.revision, revision)
        XCTAssertFalse(workspace.isDirty)
        XCTAssertFalse(workspace.canUndo)
        XCTAssertTrue(workspace.canRedo)
        XCTAssertEqual(order(workspace), Array(0..<5))
    }

    func test269PageMoveSerializesOnlyOnceAndNoOpDoesNotSerialize() throws {
        let (workspace, _) = try fixture(pageCount: 269)
        let original = try XCTUnwrap(class_getInstanceMethod(PDFDocument.self, #selector(PDFDocument.dataRepresentation)))
        let counted = try XCTUnwrap(class_getInstanceMethod(PDFDocument.self, #selector(PDFDocument.paperReorderCountedDataRepresentation)))
        method_exchangeImplementations(original, counted)
        defer { method_exchangeImplementations(counted, original) }
        ReorderSerializationCounter.reset()
        try workspace.movePages(pages: [80, 81], to: 81)
        XCTAssertEqual(ReorderSerializationCounter.value, 0)
        let pages = IndexSet([0, 80, 134, 268])
        try workspace.movePages(pages: pages, to: 269)
        XCTAssertEqual(ReorderSerializationCounter.value, 1)
        XCTAssertEqual(order(workspace), (0..<269).filter { !pages.contains($0) } + Array(pages))
        XCTAssertEqual(workspace.document?.pageCount, 269)
        XCTAssertEqual(workspace.revision, 1)
    }

    func testMovePreservesPageGeometryAnnotationsNestedOutlineAndSavedRoundTrip() throws {
        let (workspace, _) = try fixture()
        try workspace.movePages(pages: [1, 3], to: 5)
        try assertPreserved(try XCTUnwrap(workspace.document), expected: [0, 2, 4, 1, 3])
        let output = FileManager.default.temporaryDirectory.appendingPathComponent("Reordered-\(UUID().uuidString).pdf")
        addTeardownBlock { try? FileManager.default.removeItem(at: output) }
        try workspace.saveCopy(to: output)
        try assertPreserved(try XCTUnwrap(PDFDocument(url: output)), expected: [0, 2, 4, 1, 3])
        workspace.undo()
        try assertPreserved(try XCTUnwrap(workspace.document), expected: Array(0..<5))
        workspace.redo()
        try assertPreserved(try XCTUnwrap(workspace.document), expected: [0, 2, 4, 1, 3])
    }

    private func assertPreserved(_ document: PDFDocument, expected: [Int], file: StaticString = #filePath, line: UInt = #line) throws {
        XCTAssertEqual(document.pageCount, expected.count, file: file, line: line)
        XCTAssertEqual(document.documentAttributes?[PDFDocumentAttribute.titleAttribute] as? String, "Reorder fixture", file: file, line: line)
        for (index, original) in expected.enumerated() {
            let page = try XCTUnwrap(document.page(at: index), file: file, line: line)
            XCTAssertEqual(page.rotation, original % 4 * 90, file: file, line: line)
            XCTAssertEqual(page.bounds(for: .mediaBox), CGRect(x: 0, y: 0, width: 400 + original * 10, height: 600), file: file, line: line)
            XCTAssertEqual(page.bounds(for: .cropBox), CGRect(x: 10, y: 20, width: 350 + original * 10, height: 540), file: file, line: line)
            XCTAssertEqual(page.annotations.first?.contents, "page-\(original)", file: file, line: line)
        }
        let root = try XCTUnwrap(document.outlineRoot, file: file, line: line)
        let parent = try XCTUnwrap(root.child(at: 0), file: file, line: line)
        let nested = try XCTUnwrap(parent.child(at: 0), file: file, line: line)
        XCTAssertEqual(parent.label, "Parent", file: file, line: line)
        XCTAssertEqual(nested.label, "Nested", file: file, line: line)
        for (entry, original, point, zoom) in [(parent, 1, CGPoint(x: 20, y: 30), CGFloat(1.5)), (nested, 3, CGPoint(x: 40, y: 50), CGFloat(2.25))] {
            let destination = try XCTUnwrap(entry.destination, file: file, line: line)
            let page = try XCTUnwrap(destination.page, file: file, line: line)
            XCTAssertEqual(document.index(for: page), expected.firstIndex(of: original), file: file, line: line)
            XCTAssertEqual(destination.point, point, file: file, line: line)
            XCTAssertEqual(destination.zoom, zoom, file: file, line: line)
        }
    }

    private func order(_ workspace: PDFWorkspace) -> [Int] {
        guard let document = workspace.document else { return [] }
        return (0..<document.pageCount).compactMap { index in
            document.page(at: index)?.annotations.first?.contents.flatMap { Int($0.replacingOccurrences(of: "page-", with: "")) }
        }
    }

    private func fixture(pageCount: Int = 5, restricted: Bool = false) throws -> (PDFWorkspace, URL) {
        let document = PDFDocument()
        document.documentAttributes = [PDFDocumentAttribute.titleAttribute: "Reorder fixture"]
        for index in 0..<pageCount {
            let page = PDFPage()
            page.setBounds(CGRect(x: 0, y: 0, width: 400 + index * 10, height: 600), for: .mediaBox)
            page.setBounds(CGRect(x: 10, y: 20, width: 350 + index * 10, height: 540), for: .cropBox)
            page.rotation = index % 4 * 90
            let note = PDFAnnotation(bounds: CGRect(x: 30, y: 40, width: 80, height: 50), forType: .text, withProperties: nil)
            note.contents = "page-\(index)"
            page.addAnnotation(note)
            document.insert(page, at: index)
        }
        if pageCount == 5 {
            let root = PDFOutline()
            let parent = PDFOutline(); parent.label = "Parent"
            parent.destination = PDFDestination(page: try XCTUnwrap(document.page(at: 1)), at: CGPoint(x: 20, y: 30))
            parent.destination?.zoom = 1.5
            let nested = PDFOutline(); nested.label = "Nested"
            nested.destination = PDFDestination(page: try XCTUnwrap(document.page(at: 3)), at: CGPoint(x: 40, y: 50))
            nested.destination?.zoom = 2.25
            parent.insertChild(nested, at: 0); root.insertChild(parent, at: 0)
            document.outlineRoot = root
        }
        let options: [PDFDocumentWriteOption: Any] = restricted ? [.userPasswordOption: "reader", .ownerPasswordOption: "owner", .accessPermissionsOption: 0] : [:]
        let data = try XCTUnwrap(document.dataRepresentation(options: options))
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("PageReorder-\(UUID().uuidString).pdf")
        try data.write(to: url)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        let workspace = PDFWorkspace()
        if restricted { try workspace.open(url, password: "reader") }
        else { try workspace.open(url) }
        return (workspace, url)
    }
}
