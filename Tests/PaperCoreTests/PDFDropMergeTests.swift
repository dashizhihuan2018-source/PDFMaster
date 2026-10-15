import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor
final class PDFDropMergeTests: XCTestCase {
    private var directory: URL!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("PDFDropMergeTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        if let directory { try? FileManager.default.removeItem(at: directory) }
    }

    func testMultiplePDFsInsertInOrderSaveReopenAndUndoAsOneTransaction() throws {
        let base = try fixture("base.pdf", labels: ["A", "B"])
        let first = try fixture("first.pdf", labels: ["X", "Y"])
        let second = try fixture("second.pdf", labels: ["Z"])
        let originals = try [base, first, second].map { try Data(contentsOf: $0) }
        let workspace = PDFWorkspace()
        try workspace.open(base)
        try workspace.insertPDFs([first, second], at: 1)
        XCTAssertEqual(labels(workspace.document), ["A", "X", "Y", "Z", "B"])
        XCTAssertEqual(workspace.revision, 1)
        XCTAssertTrue(workspace.isDirty)
        XCTAssertEqual(workspace.sourceURL, base.standardizedFileURL)
        let copy = directory.appendingPathComponent("merged-copy.pdf")
        try workspace.saveCopy(to: copy)
        XCTAssertEqual(labels(PDFDocument(url: copy)), ["A", "X", "Y", "Z", "B"])
        XCTAssertEqual(try [base, first, second].map { try Data(contentsOf: $0) }, originals)
        XCTAssertThrowsError(try workspace.saveCopy(to: base))
        workspace.undo()
        XCTAssertEqual(labels(workspace.document), ["A", "B"])
        XCTAssertFalse(workspace.canUndo)
        XCTAssertFalse(workspace.isDirty)
        workspace.redo()
        XCTAssertEqual(labels(workspace.document), ["A", "X", "Y", "Z", "B"])
    }

    func testMalformedLaterPDFRejectsWholeBatchWithoutUndoOrRevision() throws {
        let base = try fixture("base.pdf", labels: ["A", "B"])
        let good = try fixture("good.pdf", labels: ["X"])
        let broken = directory.appendingPathComponent("broken.pdf")
        try Data("not a PDF".utf8).write(to: broken)
        let workspace = PDFWorkspace()
        try workspace.open(base)
        let before = workspace.document
        XCTAssertThrowsError(try workspace.insertPDFs([good, broken], at: 1))
        XCTAssertTrue(workspace.document === before)
        XCTAssertEqual(labels(workspace.document), ["A", "B"])
        XCTAssertEqual(workspace.revision, 0)
        XCTAssertFalse(workspace.canUndo)
        XCTAssertFalse(workspace.isDirty)
        XCTAssertEqual(workspace.sourceURL, base.standardizedFileURL)
    }

    func testEncryptedLaterPDFRejectsWholeBatchEvenWithEmptyUserPassword() throws {
        let base = try fixture("base.pdf", labels: ["A"])
        let good = try fixture("good.pdf", labels: ["X"])
        let protected = directory.appendingPathComponent("protected.pdf")
        let incoming = try XCTUnwrap(PDFDocument(url: good))
        XCTAssertTrue(incoming.write(to: protected, withOptions: [
            PDFDocumentWriteOption.userPasswordOption: "",
            PDFDocumentWriteOption.ownerPasswordOption: "owner",
            PDFDocumentWriteOption.accessPermissionsOption: 0
        ]))
        XCTAssertTrue(try XCTUnwrap(PDFDocument(url: protected)).isEncrypted)
        let workspace = PDFWorkspace()
        try workspace.open(base)
        let before = workspace.document
        XCTAssertThrowsError(try workspace.insertPDFs([good, protected], at: 1))
        XCTAssertTrue(workspace.document === before)
        XCTAssertEqual(labels(workspace.document), ["A"])
        XCTAssertEqual(workspace.revision, 0)
        XCTAssertFalse(workspace.canUndo)
    }

    func testAppendAndBeginningSlotsAndInvalidBatchAreTransactional() throws {
        let base = try fixture("base.pdf", labels: ["A", "B"])
        let incoming = try fixture("incoming.pdf", labels: ["X"])
        let workspace = PDFWorkspace()
        try workspace.open(base)
        try workspace.insertPDFs([incoming], at: 2)
        XCTAssertEqual(labels(workspace.document), ["A", "B", "X"])
        workspace.undo()
        try workspace.insertPDFs([incoming], at: 0)
        XCTAssertEqual(labels(workspace.document), ["X", "A", "B"])
        workspace.undo()
        let before = workspace.document
        XCTAssertThrowsError(try workspace.insertPDFs([], at: 1))
        XCTAssertThrowsError(try workspace.insertPDFs([incoming], at: -1))
        XCTAssertThrowsError(try workspace.insertPDFs([incoming], at: 3))
        XCTAssertTrue(workspace.document === before)
        XCTAssertFalse(workspace.canUndo)
    }

    private func fixture(_ name: String, labels: [String]) throws -> URL {
        let document = PDFDocument()
        for (index, label) in labels.enumerated() {
            let page = PDFPage()
            page.setBounds(CGRect(x: 0, y: 0, width: 612, height: 792), for: .mediaBox)
            let marker = PDFAnnotation(bounds: CGRect(x: 30, y: 30, width: 120, height: 30), forType: .freeText, withProperties: nil)
            marker.contents = label
            page.addAnnotation(marker)
            document.insert(page, at: index)
        }
        let url = directory.appendingPathComponent(name)
        XCTAssertTrue(document.write(to: url))
        return url
    }

    private func labels(_ document: PDFDocument?) -> [String] {
        guard let document else { return [] }
        return (0..<document.pageCount).map { document.page(at: $0)?.annotations.first?.contents ?? "<missing>" }
    }
}
