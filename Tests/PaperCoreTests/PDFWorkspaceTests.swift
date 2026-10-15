import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor
final class PDFWorkspaceTests: XCTestCase {
    private var temporaryDirectory: URL!

    override func setUpWithError() throws {
        temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("PaperCoreTests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: temporaryDirectory, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        if temporaryDirectory != nil {
            try? FileManager.default.removeItem(at: temporaryDirectory)
        }
    }

    func testPageRangeParsesOneBasedRangesAndRejectsInvalidInput() throws {
        XCTAssertEqual(try PageRange.parse("1, 3-5, 7", count: 7), IndexSet([0, 2, 3, 4, 6]))
        XCTAssertEqual(try PageRange.parse("3-1, 2", count: 3), IndexSet([0, 1, 2]))
        for invalid in ["", "0", "4", "1-x", "1,,2", "2-"] {
            XCTAssertThrowsError(try PageRange.parse(invalid, count: 3), "Expected rejection for \(invalid)")
        }
    }

    func testCreateBlankProducesOneCleanUSLetterPage() throws {
        let workspace = PDFWorkspace()
        try workspace.createBlank()

        XCTAssertEqual(workspace.document?.pageCount, 1)
        XCTAssertEqual(workspace.document?.page(at: 0)?.bounds(for: .mediaBox).size.width ?? 0, 612, accuracy: 0.1)
        XCTAssertEqual(workspace.document?.page(at: 0)?.bounds(for: .mediaBox).size.height ?? 0, 792, accuracy: 0.1)
        XCTAssertNil(workspace.sourceURL)
        XCTAssertFalse(workspace.isDirty)
        XCTAssertEqual(workspace.revision, 0)
    }

    func testOpenCopiesDocumentAndSaveCopyNeverOverwritesSourceOrAliases() throws {
        let source = try makePDF(named: "source.pdf", labels: ["A", "B"])
        let originalBytes = try Data(contentsOf: source)
        let workspace = PDFWorkspace()
        try workspace.open(source)
        try workspace.rotate(page: 0, degrees: 90)

        XCTAssertEqual(try Data(contentsOf: source), originalBytes)
        XCTAssertThrowsError(try workspace.saveCopy(to: source))

        let symlink = temporaryDirectory.appendingPathComponent("source-link.pdf")
        try FileManager.default.createSymbolicLink(at: symlink, withDestinationURL: source)
        XCTAssertThrowsError(try workspace.saveCopy(to: symlink))

        let hardlink = temporaryDirectory.appendingPathComponent("source-hard.pdf")
        try FileManager.default.linkItem(at: source, to: hardlink)
        XCTAssertThrowsError(try workspace.saveCopy(to: hardlink))
        XCTAssertEqual(try Data(contentsOf: source), originalBytes)

        let copy = temporaryDirectory.appendingPathComponent("copy.pdf")
        try workspace.saveCopy(to: copy)
        XCTAssertFalse(workspace.isDirty)
        XCTAssertEqual(workspace.sourceURL, source.standardizedFileURL)
        XCTAssertEqual(PDFDocument(url: copy)?.page(at: 0)?.rotation, 90)
        XCTAssertThrowsError(try workspace.saveCopy(to: source))
    }

    func testRotateMoveDeleteAndInvalidMutationsAreTransactional() throws {
        let source = try makePDF(named: "pages.pdf", labels: ["A", "B", "C"])
        let workspace = PDFWorkspace()
        try workspace.open(source)
        try workspace.rotate(page: 1, degrees: -90)
        XCTAssertEqual(workspace.document?.page(at: 1)?.rotation, 270)

        try workspace.movePage(from: 2, to: 0)
        XCTAssertEqual(pageLabel(workspace.document, at: 0), "C")
        try workspace.delete(pages: IndexSet(integer: 1))
        XCTAssertEqual(workspace.document?.pageCount, 2)

        let before = workspace.document!
        let revision = workspace.revision
        XCTAssertThrowsError(try workspace.delete(pages: IndexSet([0, 1])))
        XCTAssertThrowsError(try workspace.rotate(page: 8, degrees: 90))
        XCTAssertThrowsError(try workspace.movePage(from: 0, to: 8))
        XCTAssertTrue(workspace.document === before)
        XCTAssertEqual(workspace.revision, revision)
    }

    func testInsertBlankInsertPDFAndExtractPersistExpectedPageOrder() throws {
        let source = try makePDF(named: "base.pdf", labels: ["A", "B"])
        let addition = try makePDF(named: "addition.pdf", labels: ["X", "Y"])
        let workspace = PDFWorkspace()
        try workspace.open(source)
        try workspace.insertBlank(at: 1)
        XCTAssertEqual(workspace.document?.pageCount, 3)
        XCTAssertNil(pageLabel(workspace.document, at: 1))

        try workspace.insertPDF(addition, at: 2)
        XCTAssertEqual((0..<5).map { pageLabel(workspace.document, at: $0) }, ["A", nil, "X", "Y", "B"])

        let extracted = temporaryDirectory.appendingPathComponent("extract.pdf")
        try workspace.extract(pages: IndexSet([0, 2, 4]), to: extracted)
        let reopened = PDFDocument(url: extracted)
        XCTAssertEqual((0..<3).map { pageLabel(reopened, at: $0) }, ["A", "X", "B"])
        XCTAssertTrue(workspace.isDirty)

        let before = workspace.document!
        XCTAssertThrowsError(try workspace.insertPDF(source, at: 99))
        XCTAssertThrowsError(try workspace.extract(pages: IndexSet(integer: 99), to: extracted))
        XCTAssertTrue(workspace.document === before)
    }

    func testTextMarkupAndInkSurviveSaveAndReopen() throws {
        let source = try makePDF(named: "annotate.pdf", labels: ["Selectable text"])
        let workspace = PDFWorkspace()
        try workspace.open(source)
        try workspace.addText("Note", page: 0, bounds: CGRect(x: 40, y: 40, width: 100, height: 30))

        let selection = workspace.document!.findString("Selectable", withOptions: []).first!
        try workspace.addMarkup(selection: selection, kind: .highlight, color: .yellow)
        try workspace.addInk(
            points: [CGPoint(x: 20, y: 20), CGPoint(x: 60, y: 70), CGPoint(x: 100, y: 30)],
            page: 0,
            color: .red,
            width: 4
        )

        let output = temporaryDirectory.appendingPathComponent("annotated.pdf")
        try workspace.saveCopy(to: output)
        let annotations = PDFDocument(url: output)!.page(at: 0)!.annotations
        let persistedTypes = annotations.map(\.type)
        XCTAssertTrue(annotations.contains { $0.type == "FreeText" && $0.contents == "Note" }, "\(persistedTypes)")
        XCTAssertTrue(annotations.contains { $0.type == "Highlight" }, "\(persistedTypes)")
        XCTAssertTrue(annotations.contains { $0.type == "Ink" }, "\(persistedTypes)")
        let ink = try XCTUnwrap(annotations.first { $0.type == "Ink" })
        XCTAssertEqual(ink.bounds.minX, 16, accuracy: 0.1)
        XCTAssertEqual(ink.bounds.minY, 16, accuracy: 0.1)
        XCTAssertEqual(ink.bounds.maxX, 104, accuracy: 0.1)
        XCTAssertEqual(ink.bounds.maxY, 74, accuracy: 0.1)
        let persistedPath = try XCTUnwrap(ink.paths?.first)
        XCTAssertEqual(persistedPath.bounds.minX, 4, accuracy: 0.1)
        XCTAssertEqual(persistedPath.bounds.minY, 4, accuracy: 0.1)
        XCTAssertEqual(persistedPath.bounds.maxX, 84, accuracy: 0.1)
        XCTAssertEqual(persistedPath.bounds.maxY, 54, accuracy: 0.1)
    }

    func testMultiStrokeInkIsOneTransactionAndInvalidStrokeRollsBack() throws {
        let workspace = PDFWorkspace()
        try workspace.createBlank()
        let strokes = [
            [CGPoint(x: 20, y: 20), CGPoint(x: 80, y: 40)],
            [CGPoint(x: 30, y: 90), CGPoint(x: 70, y: 120), CGPoint(x: 110, y: 95)]
        ]

        try workspace.addInk(strokes: strokes, page: 0, color: .black, width: 3)
        XCTAssertEqual(workspace.revision, 1)
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.filter { $0.type == "Ink" }.count, 2)

        workspace.undo()
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.filter { $0.type == "Ink" }.count, 0)

        let before = workspace.document
        let revision = workspace.revision
        XCTAssertThrowsError(try workspace.addInk(
            strokes: [strokes[0], [CGPoint(x: 1, y: 1)]],
            page: 0,
            color: .black,
            width: 3
        )) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .invalidInk)
        }
        XCTAssertTrue(workspace.document === before)
        XCTAssertEqual(workspace.revision, revision)
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.filter { $0.type == "Ink" }.count, 0)
    }

    func testMarkupRejectsSelectionFromAnotherDocumentWithoutChangingState() throws {
        let source = try makePDF(named: "current.pdf", labels: ["Current"])
        let other = try makePDF(named: "other.pdf", labels: ["Other"])
        let workspace = PDFWorkspace()
        try workspace.open(source)
        let foreignSelection = PDFDocument(url: other)!.findString("Other", withOptions: []).first!
        let before = workspace.document!
        XCTAssertThrowsError(try workspace.addMarkup(selection: foreignSelection, kind: .underline, color: .blue))
        XCTAssertTrue(workspace.document === before)
        XCTAssertEqual(workspace.revision, 0)
    }

    func testMarkupPreservesMultilineAndDiscontiguousSelectionFragments() throws {
        let source = try makePDF(
            named: "fragmented.pdf",
            pages: [["First selected", "MIDDLE MUST STAY CLEAR", "Third selected"]]
        )
        let workspace = PDFWorkspace()
        try workspace.open(source)
        let document = try XCTUnwrap(workspace.document)
        let first = try XCTUnwrap(document.findString("First selected", withOptions: []).first)
        let third = try XCTUnwrap(document.findString("Third selected", withOptions: []).first)
        let discontiguous = PDFSelection(document: document)
        discontiguous.add([first, third])

        try workspace.addMarkup(selection: discontiguous, kind: .highlight, color: .yellow)
        let output = temporaryDirectory.appendingPathComponent("fragmented-output.pdf")
        try workspace.saveCopy(to: output)
        let page = try XCTUnwrap(PDFDocument(url: output)?.page(at: 0))
        let highlights = page.annotations.filter { $0.type == "Highlight" }.sorted { $0.bounds.minY > $1.bounds.minY }

        XCTAssertEqual(highlights.count, 2)
        let middle = try XCTUnwrap(document.findString("MIDDLE MUST STAY CLEAR", withOptions: []).first)
        let middleBounds = middle.bounds(for: try XCTUnwrap(document.page(at: 0)))
        XCTAssertFalse(highlights.contains { $0.bounds.intersects(middleBounds) })
        if highlights.count == 2 {
            XCTAssertEqual(highlights[0].quadrilateralPoints?.count, 4)
            XCTAssertEqual(highlights[1].quadrilateralPoints?.count, 4)
            XCTAssertGreaterThan(highlights[0].bounds.minY - highlights[1].bounds.maxY, 20)
        }
    }

    func testRestrictedCopyingAssemblyAndCommentingAreDeniedIndependentlyWithRollback() throws {
        let copyRestricted = try makeRestrictedPDF(named: "no-copy.pdf", denied: .allowsContentCopying)
        let assemblyRestricted = try makeRestrictedPDF(named: "no-assembly.pdf", denied: .allowsDocumentAssembly)
        let commentRestricted = try makeRestrictedPDF(named: "no-comment.pdf", denied: .allowsCommenting)
        let workspace = PDFWorkspace()

        try workspace.open(copyRestricted)
        let copyDocument = workspace.document
        let extracted = temporaryDirectory.appendingPathComponent("forbidden-extract.pdf")
        XCTAssertThrowsError(try workspace.extract(pages: IndexSet(integer: 0), to: extracted)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied)
        }
        XCTAssertTrue(workspace.document === copyDocument)
        XCTAssertEqual(workspace.revision, 0)
        XCTAssertFalse(FileManager.default.fileExists(atPath: extracted.path))

        try workspace.open(assemblyRestricted)
        let assemblyDocument = workspace.document
        XCTAssertThrowsError(try workspace.rotate(page: 0, degrees: 90)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied)
        }
        XCTAssertTrue(workspace.document === assemblyDocument)
        XCTAssertEqual(workspace.revision, 0)

        try workspace.open(commentRestricted)
        let commentDocument = workspace.document
        XCTAssertThrowsError(try workspace.addText("Forbidden", page: 0, bounds: CGRect(x: 10, y: 10, width: 80, height: 20))) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied)
        }
        XCTAssertTrue(workspace.document === commentDocument)
        XCTAssertEqual(workspace.revision, 0)
    }

    func testFailedSavePreservesExistingDestinationBytesAndDirtyState() throws {
        let workspace = PDFWorkspace()
        try workspace.createBlank()
        try workspace.rotate(page: 0, degrees: 90)
        let lockedDirectory = temporaryDirectory.appendingPathComponent("locked-directory", isDirectory: true)
        try FileManager.default.createDirectory(at: lockedDirectory, withIntermediateDirectories: true)
        let destination = lockedDirectory.appendingPathComponent("existing.pdf")
        let originalDestination = Data("existing destination must survive".utf8)
        try originalDestination.write(to: destination)
        try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: lockedDirectory.path)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: lockedDirectory.path) }

        XCTAssertThrowsError(try workspace.saveCopy(to: destination))
        XCTAssertEqual(try Data(contentsOf: destination), originalDestination)
        XCTAssertTrue(workspace.isDirty)
        XCTAssertEqual(workspace.revision, 1)
    }

    func testUndoRedoUsesDocumentSnapshotsAndNewMutationClearsRedo() throws {
        let workspace = PDFWorkspace()
        try workspace.createBlank()
        try workspace.insertBlank(at: 1)
        try workspace.rotate(page: 0, degrees: 90)
        XCTAssertEqual(workspace.document?.pageCount, 2)
        XCTAssertEqual(workspace.document?.page(at: 0)?.rotation, 90)
        XCTAssertTrue(workspace.canUndo)

        workspace.undo()
        XCTAssertEqual(workspace.document?.page(at: 0)?.rotation, 0)
        XCTAssertTrue(workspace.canRedo)
        workspace.undo()
        XCTAssertEqual(workspace.document?.pageCount, 1)
        workspace.redo()
        XCTAssertEqual(workspace.document?.pageCount, 2)
        try workspace.insertBlank(at: 0)
        XCTAssertFalse(workspace.canRedo)
        XCTAssertEqual(workspace.document?.pageCount, 3)
    }

    func testLockedAndMalformedPDFsFailOpenWithoutReplacingCurrentDocument() throws {
        let valid = try makePDF(named: "valid.pdf", labels: ["Safe"])
        let workspace = PDFWorkspace()
        try workspace.open(valid)
        let before = workspace.document!
        let beforeData = before.dataRepresentation()!

        let malformed = temporaryDirectory.appendingPathComponent("broken.pdf")
        try Data("not a pdf".utf8).write(to: malformed)
        XCTAssertThrowsError(try workspace.open(malformed))
        XCTAssertTrue(workspace.document === before)
        XCTAssertEqual(workspace.sourceURL, valid.standardizedFileURL)

        let locked = temporaryDirectory.appendingPathComponent("locked.pdf")
        let document = PDFDocument(data: beforeData)!
        XCTAssertTrue(document.write(
            to: locked,
            withOptions: [
                PDFDocumentWriteOption.userPasswordOption: "secret",
                PDFDocumentWriteOption.ownerPasswordOption: "owner"
            ]
        ))
        XCTAssertThrowsError(try workspace.open(locked))
        XCTAssertTrue(workspace.document === before)
    }

    private func makePDF(named name: String, labels: [String]) throws -> URL {
        try makePDF(named: name, pages: labels.map { [$0] })
    }

    private func makePDF(named name: String, pages: [[String]]) throws -> URL {
        let url = temporaryDirectory.appendingPathComponent(name)
        guard let consumer = CGDataConsumer(url: url as CFURL) else { throw FixtureError.writeFailed }
        var mediaBox = CGRect(x: 0, y: 0, width: 612, height: 792)
        guard let context = CGContext(consumer: consumer, mediaBox: &mediaBox, nil) else {
            throw FixtureError.writeFailed
        }
        for lines in pages {
            context.beginPDFPage(nil)
            context.saveGState()
            let graphicsContext = NSGraphicsContext(cgContext: context, flipped: false)
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = graphicsContext
            for (lineIndex, line) in lines.enumerated() {
                (line as NSString).draw(
                    at: NSPoint(x: 72, y: 700 - CGFloat(lineIndex * 60)),
                    withAttributes: [.font: NSFont.systemFont(ofSize: 18), .foregroundColor: NSColor.black]
                )
            }
            NSGraphicsContext.restoreGraphicsState()
            context.restoreGState()
            context.endPDFPage()
        }
        context.closePDF()
        guard PDFDocument(url: url)?.pageCount == pages.count else { throw FixtureError.writeFailed }
        return url
    }

    private func makeRestrictedPDF(named name: String, denied: PDFAccessPermissions) throws -> URL {
        let plain = try makePDF(named: "plain-\(name)", labels: ["Restricted fixture"])
        let document = try XCTUnwrap(PDFDocument(url: plain))
        let independentlyControllable: [PDFAccessPermissions] = [
            .allowsLowQualityPrinting,
            .allowsHighQualityPrinting,
            .allowsDocumentAssembly,
            .allowsContentCopying,
            .allowsContentAccessibility,
            .allowsCommenting,
            .allowsFormFieldEntry
        ]
        let allowedBits = independentlyControllable
            .filter { $0 != denied }
            .reduce(UInt(0)) { $0 | $1.rawValue }
        let output = temporaryDirectory.appendingPathComponent(name)
        XCTAssertTrue(document.write(
            to: output,
            withOptions: [
                PDFDocumentWriteOption.ownerPasswordOption: "owner-only",
                PDFDocumentWriteOption.accessPermissionsOption: NSNumber(value: allowedBits)
            ]
        ))
        let reopened = try XCTUnwrap(PDFDocument(url: output))
        XCTAssertFalse(reopened.isLocked)
        return output
    }

    private func pageLabel(_ document: PDFDocument?, at index: Int) -> String? {
        document?.page(at: index)?.string?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
    }

    private enum FixtureError: Error { case writeFailed }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
