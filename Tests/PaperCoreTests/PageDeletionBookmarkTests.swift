import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor final class PageDeletionBookmarkTests: XCTestCase {
    private func fixture(nested: Bool = false) throws -> (PDFWorkspace, URL, Data) {
        let document = PDFDocument()
        for index in 0..<4 {
            let page = PDFPage()
            page.setBounds(CGRect(x: 0, y: 0, width: 420, height: 594), for: .mediaBox)
            let note = PDFAnnotation(bounds: CGRect(x: 20, y: 40, width: 100, height: 30), forType: .freeText, withProperties: nil)
            note.contents = "PAGE-\(index)"; page.addAnnotation(note)
            document.insert(page, at: index)
        }
        let root = PDFOutline()
        for (label, page) in [("First", 0), ("Deleted target", 1), ("Survivor", 3)] {
            let item = PDFOutline(); item.label = label
            item.destination = PDFDestination(page: document.page(at: page)!, at: CGPoint(x: 25, y: 300))
            root.insertChild(item, at: root.numberOfChildren)
        }
        document.outlineRoot = root
        if nested {
            let child = PDFOutline(); child.label = "Nested deleted target"
            child.destination = PDFDestination(page: document.page(at: 2)!, at: .zero)
            root.child(at: 0)!.insertChild(child, at: 0)
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("PageDeletion-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appendingPathComponent("bookmarked.pdf")
        let data = try XCTUnwrap(document.dataRepresentation()); try data.write(to: url)
        let workspace = PDFWorkspace(); try workspace.open(url)
        return (workspace, url, data)
    }

    func testBookmarkedPageCanBeDeletedSavedAndUndoneWithoutChangingSource() throws {
        let (workspace, source, bytes) = try fixture()
        try workspace.delete(pages: [1])
        XCTAssertEqual(workspace.document?.pageCount, 3)
        XCTAssertEqual((0..<3).map { workspace.document?.page(at: $0)?.annotations.first?.contents }, ["PAGE-0", "PAGE-2", "PAGE-3"])
        // Every retained bookmark must resolve to a surviving page, including after serialization.
        let copy = source.deletingLastPathComponent().appendingPathComponent("deleted-copy.pdf")
        try workspace.saveCopy(to: copy)
        let reopened = try XCTUnwrap(PDFDocument(url: copy))
        XCTAssertEqual(reopened.pageCount, 3)
        let root = try XCTUnwrap(reopened.outlineRoot)
        XCTAssertEqual(root.numberOfChildren, 2)
        XCTAssertEqual((0..<root.numberOfChildren).compactMap { root.child(at: $0)?.label }, ["First", "Survivor"])
        for index in 0..<root.numberOfChildren {
            let target = try XCTUnwrap(root.child(at: index)?.destination?.page)
            XCTAssertNotEqual(reopened.index(for: target), NSNotFound)
        }
        let survivor = (0..<root.numberOfChildren).compactMap { root.child(at: $0) }.first { $0.label == "Survivor" }
        XCTAssertEqual(reopened.index(for: try XCTUnwrap(survivor?.destination?.page)), 2)
        XCTAssertEqual(workspace.undo(), .document)
        XCTAssertEqual(workspace.document?.pageCount, 4)
        XCTAssertEqual(workspace.document?.outlineRoot?.numberOfChildren, 3)
        XCTAssertEqual(workspace.redo(), .document)
        XCTAssertEqual(workspace.document?.pageCount, 3)
        XCTAssertEqual(try Data(contentsOf: source), bytes)
    }

    func testNestedBookmarksToDeletedPagesAreRemovedAndSurvivorsKeepTheirTargets() throws {
        let (workspace, _, _) = try fixture(nested: true)
        XCTAssertEqual(workspace.document?.outlineRoot?.child(at: 0)?.numberOfChildren, 1)
        try workspace.delete(pages: [1, 2])
        XCTAssertEqual(workspace.document?.pageCount, 2)
        XCTAssertEqual(workspace.document?.outlineRoot?.numberOfChildren, 2)
        XCTAssertEqual(workspace.document?.outlineRoot?.child(at: 0)?.numberOfChildren, 0)
        let last = try XCTUnwrap(workspace.document?.outlineRoot?.child(at: 1)?.destination?.page)
        XCTAssertEqual(workspace.document?.index(for: last), 1)
        workspace.undo()
        XCTAssertEqual(workspace.document?.outlineRoot?.child(at: 0)?.numberOfChildren, 1)
    }

    func testDeletingAllPagesIsRejectedWithoutChangingDocumentOrHistory() throws {
        let (workspace, _, _) = try fixture(nested: true)
        let original = workspace.document, revision = workspace.revision
        XCTAssertThrowsError(try workspace.delete(pages: IndexSet(integersIn: 0..<4))) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .cannotDeleteLastPage)
        }
        XCTAssertTrue(workspace.document === original)
        XCTAssertEqual(workspace.revision, revision)
        XCTAssertFalse(workspace.canUndo)
        XCTAssertEqual(workspace.document?.outlineRoot?.numberOfChildren, 3)
    }
}
