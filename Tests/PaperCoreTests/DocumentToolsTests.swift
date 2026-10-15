import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor
final class DocumentToolsTests: XCTestCase {
    private var directory: URL!

    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("DocumentTools-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws { try? FileManager.default.removeItem(at: directory) }

    func testShapeAnnotationsPersistSubtypeStylesAndCanBeEditedRemovedAndUndone() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        let style = AnnotationStyle(strokeColor: .red, fillColor: .yellow, lineWidth: 3)
        try workspace.addPopupNote("Remember", author: "Ada", page: 0, bounds: CGRect(x: 20, y: 30, width: 24, height: 24), color: .blue)
        try workspace.addShape(.rectangle, page: 0, bounds: CGRect(x: 50, y: 60, width: 100, height: 80), style: style)
        try workspace.addShape(.ellipse, page: 0, bounds: CGRect(x: 180, y: 60, width: 100, height: 80), style: style)
        try workspace.addLine(from: CGPoint(x: 30, y: 200), to: CGPoint(x: 180, y: 240), page: 0, style: style, endings: .arrow)
        try workspace.addLine(from: CGPoint(x: 30, y: 280), to: CGPoint(x: 180, y: 320), page: 0, style: style, endings: .doubleArrow)

        let page = try XCTUnwrap(workspace.document?.page(at: 0))
        let squareIndex = try XCTUnwrap(page.annotations.firstIndex { $0.type == "Square" })
        let circleIndex = try XCTUnwrap(page.annotations.firstIndex { $0.type == "Circle" })
        try workspace.editAnnotation(page: 0, index: squareIndex, contents: "Box", bounds: CGRect(x: 55, y: 65, width: 110, height: 90), color: .green)
        try workspace.removeAnnotation(page: 0, index: circleIndex)
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.filter { $0.type != "Popup" }.count, 4)
        workspace.undo()
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.filter { $0.type != "Popup" }.count, 5)

        let output = directory.appendingPathComponent("shapes.pdf"); try workspace.saveCopy(to: output)
        let annotations = try XCTUnwrap(PDFDocument(url: output)?.page(at: 0)?.annotations)
        XCTAssertTrue(annotations.contains { $0.type == "Text" && $0.contents == "Remember" && $0.userName == "Ada" })
        let square = try XCTUnwrap(annotations.first { $0.type == "Square" })
        XCTAssertEqual(square.contents, "Box")
        XCTAssertEqual(square.bounds.width, 110, accuracy: 0.1)
        let lines = annotations.filter { $0.type == "Line" }
        XCTAssertEqual(lines[0].endLineStyle, .openArrow)
        XCTAssertEqual(lines[1].startLineStyle, .openArrow)
        XCTAssertEqual(lines[1].endLineStyle, .openArrow)
    }

    func testInvalidGeometryAndAnnotationIndexesRollback() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        let before = workspace.document; let revision = workspace.revision
        XCTAssertThrowsError(try workspace.addShape(.rectangle, page: 0, bounds: CGRect(x: CGFloat.nan, y: 0, width: 10, height: 10), style: .default))
        XCTAssertThrowsError(try workspace.addLine(from: .zero, to: .zero, page: 0, style: .default, endings: .none))
        XCTAssertThrowsError(try workspace.removeAnnotation(page: 0, index: 0))
        XCTAssertTrue(workspace.document === before)
        XCTAssertEqual(workspace.revision, revision)
    }

    func testDuplicateAndCropPreserveContentAnnotationsAndOriginalBytes() throws {
        let source = try makePDF("source.pdf", text: "Vector source text")
        let original = try Data(contentsOf: source)
        let workspace = PDFWorkspace(); try workspace.open(source)
        try workspace.addPopupNote("note", author: nil, page: 0, bounds: CGRect(x: 40, y: 40, width: 20, height: 20), color: .yellow)
        try workspace.duplicatePage(at: 0, insertingAt: 1)
        XCTAssertEqual(workspace.document?.pageCount, 2)
        XCTAssertTrue(workspace.document?.page(at: 1)?.string?.contains("Vector source text") == true)
        XCTAssertEqual(workspace.document?.page(at: 1)?.annotations.count, 1)
        try workspace.cropPage(at: 1, to: CGRect(x: 20, y: 20, width: 300, height: 400))
        XCTAssertEqual(workspace.document?.page(at: 1)?.bounds(for: .cropBox), CGRect(x: 20, y: 20, width: 300, height: 400))
        XCTAssertEqual(try Data(contentsOf: source), original)
        let revision = workspace.revision
        XCTAssertThrowsError(try workspace.cropPage(at: 1, to: CGRect(x: 600, y: 700, width: 100, height: 100)))
        XCTAssertEqual(workspace.revision, revision)
    }

    func testTopLevelBookmarksPersistAndNestedOutlineIsPreserved() throws {
        let source = try makeOutlinedPDF()
        let workspace = PDFWorkspace(); try workspace.open(source)
        XCTAssertEqual(workspace.document?.outlineRoot?.child(at: 0)?.numberOfChildren, 1)
        try workspace.addBookmark(label: "Added", page: 1)
        XCTAssertEqual(workspace.document?.outlineRoot?.numberOfChildren, 2)
        try workspace.renameBookmark(at: 0, label: "Existing renamed")
        XCTAssertEqual(workspace.document?.outlineRoot?.numberOfChildren, 2)
        try workspace.removeBookmark(at: 1)
        XCTAssertEqual(workspace.document?.outlineRoot?.child(at: 0)?.numberOfChildren, 1)
        let output = directory.appendingPathComponent("outline-output.pdf"); try workspace.saveCopy(to: output)
        let reopened = try XCTUnwrap(PDFDocument(url: output))
        let root = try XCTUnwrap(reopened.outlineRoot)
        XCTAssertEqual(root.numberOfChildren, 1)
        XCTAssertEqual(root.child(at: 0)?.label, "Existing renamed")
        XCTAssertEqual(root.child(at: 0)?.numberOfChildren, 1)
        XCTAssertEqual(root.child(at: 0)?.child(at: 0)?.label, "Nested")
        XCTAssertEqual(root.child(at: 0)?.destination?.page?.string?.trimmingCharacters(in: .whitespacesAndNewlines), "One")
        XCTAssertEqual(root.child(at: 0)?.destination?.zoom ?? 0, 1.75, accuracy: 0.001)
        XCTAssertEqual(root.child(at: 0)?.child(at: 0)?.destination?.zoom ?? 0, 2.25, accuracy: 0.001)
        withExtendedLifetime(reopened) {}
        let revision = workspace.revision
        XCTAssertThrowsError(try workspace.addBookmark(label: "Bad", page: 9))
        XCTAssertThrowsError(try workspace.renameBookmark(at: 9, label: "Bad"))
        XCTAssertEqual(workspace.revision, revision)
    }

    func testBookmarkEditsUndoRedoAndPageOperationsKeepOriginalDestinations() throws {
        let source = try makeOutlinedPDF()
        let workspace = PDFWorkspace(); try workspace.open(source)

        try workspace.renameBookmark(at: 0, label: "Renamed")
        XCTAssertEqual(workspace.document?.outlineRoot?.child(at: 0)?.label, "Renamed")
        workspace.undo()
        XCTAssertEqual(workspace.document?.outlineRoot?.child(at: 0)?.label, "Existing")
        workspace.redo()
        XCTAssertEqual(workspace.document?.outlineRoot?.child(at: 0)?.label, "Renamed")

        try workspace.insertBlank(at: 0)
        try workspace.movePage(from: 2, to: 0)
        try workspace.duplicatePage(at: 1, insertingAt: 2)
        let output = directory.appendingPathComponent("outline-page-operations.pdf")
        try workspace.saveCopy(to: output)

        let reopened = try XCTUnwrap(PDFDocument(url: output))
        let root = try XCTUnwrap(reopened.outlineRoot)
        XCTAssertEqual(root.child(at: 0)?.destination?.page?.string?.trimmingCharacters(in: .whitespacesAndNewlines), "One")
        XCTAssertEqual(root.child(at: 0)?.child(at: 0)?.destination?.page?.string?.trimmingCharacters(in: .whitespacesAndNewlines), "Two")
        withExtendedLifetime(reopened) {}

        let revision = workspace.revision
        let bookmarkedPage = try XCTUnwrap(workspace.document?.outlineRoot?.child(at: 0)?.destination?.page)
        let bookmarkedIndex = try XCTUnwrap(workspace.document).index(for: bookmarkedPage)
        XCTAssertThrowsError(try workspace.delete(pages: IndexSet(integer: bookmarkedIndex)))
        XCTAssertEqual(workspace.revision, revision)
    }

    func testSaveAndSnapshotsPreserveEmbeddedFilesAndPageLabels() throws {
        let source = try makeCatalogPDF()
        let workspace = PDFWorkspace(); try workspace.open(source)
        try workspace.addShape(.rectangle, page: 0, bounds: CGRect(x: 30, y: 30, width: 40, height: 40), style: .default)
        workspace.undo()
        workspace.redo()
        let output = directory.appendingPathComponent("catalog-copy.pdf")
        try workspace.saveCopy(to: output)

        let catalog = try catalogFeatures(at: output)
        XCTAssertEqual(catalog.pageLabelPrefix, "A-")
        XCTAssertEqual(catalog.embeddedFileName, "generated.txt")
        XCTAssertEqual(catalog.embeddedFileContents, Data("catalog attachment".utf8))
    }

    func testBookmarkMutationFailsClosedForUnsupportedCatalogStructures() throws {
        let source = try makeCatalogPDF()
        let workspace = PDFWorkspace(); try workspace.open(source)
        let revision = workspace.revision
        XCTAssertThrowsError(try workspace.addBookmark(label: "Unsafe", page: 0))
        XCTAssertEqual(workspace.revision, revision)
        let output = directory.appendingPathComponent("catalog-unchanged.pdf")
        try workspace.saveCopy(to: output)
        let catalog = try catalogFeatures(at: output)
        XCTAssertEqual(catalog.pageLabelPrefix, "A-")
        XCTAssertEqual(catalog.embeddedFileName, "generated.txt")
    }

    func testTextAndAnnotationSummaryExportsEscapeFormatsWithoutSourcePath() throws {
        let source = try makePDF("private-name.pdf", text: "Hello <world> & all")
        let workspace = PDFWorkspace(); try workspace.open(source)
        try workspace.addPopupNote("Use **care** <script>alert(1)</script> & <now>", author: "<img src=x onerror=alert(2)> A&B", page: 0, bounds: CGRect(x: 20, y: 20, width: 20, height: 20), color: .yellow)
        XCTAssertTrue(try workspace.exportPlainText().contains("Hello <world> & all"))
        let plain = try workspace.exportAnnotationSummary(format: .plainText)
        let markdown = try workspace.exportAnnotationSummary(format: .markdown)
        let html = String(decoding: try workspace.exportAnnotationSummaryData(format: .html), as: UTF8.self)
        XCTAssertTrue(plain.contains("Use **care** <script>alert(1)</script> & <now>"))
        XCTAssertTrue(markdown.contains("Use \\*\\*care\\*\\* &lt;script&gt;alert(1)&lt;/script&gt; &amp; &lt;now&gt;"))
        XCTAssertTrue(markdown.contains("&lt;img src=x onerror=alert(2)&gt; A&amp;B"))
        XCTAssertFalse(markdown.contains("<script>"))
        XCTAssertFalse(markdown.contains("<img"))
        XCTAssertTrue(html.contains("Use **care** &lt;script&gt;alert(1)&lt;/script&gt; &amp; &lt;now&gt;"))
        XCTAssertTrue(html.contains("&lt;img src=x onerror=alert(2)&gt; A&amp;B"))
        for value in [plain, markdown, html] { XCTAssertFalse(value.contains(source.path)) }
    }

    func testFlattenedCopyKeepsVectorTextBurnsAnnotationsAndProtectsAliases() throws {
        let source = try makePDF("flatten-source.pdf", text: "Keep vector text")
        let workspace = PDFWorkspace(); try workspace.open(source)
        try workspace.addShape(.rectangle, page: 0, bounds: CGRect(x: 60, y: 60, width: 100, height: 80), style: .init(strokeColor: .red, fillColor: nil, lineWidth: 2))
        let output = directory.appendingPathComponent("flattened.pdf")
        try workspace.saveFlattenedCopy(to: output)
        let reopened = try XCTUnwrap(PDFDocument(url: output))
        XCTAssertTrue(reopened.string?.contains("Keep vector text") == true)
        XCTAssertTrue(reopened.page(at: 0)?.annotations.isEmpty == true)
        let sourceDocument = try XCTUnwrap(PDFDocument(url: source))
        let sourceRender = try XCTUnwrap(sourceDocument.page(at: 0)?.thumbnail(of: NSSize(width: 612, height: 792), for: .mediaBox).tiffRepresentation)
        let flattenedRender = try XCTUnwrap(reopened.page(at: 0)?.thumbnail(of: NSSize(width: 612, height: 792), for: .mediaBox).tiffRepresentation)
        XCTAssertNotEqual(sourceRender, flattenedRender, "Burned-in rectangle must change rendered pixels")
        XCTAssertThrowsError(try workspace.saveFlattenedCopy(to: source))
    }

    func testKnownPasswordOpenRetainsConservativePermissionsAndRejectsAllPDFCopies() throws {
        let plain = try makePDF("plain.pdf", text: "Secret text")
        let protected = directory.appendingPathComponent("protected.pdf")
        let document = try XCTUnwrap(PDFDocument(url: plain))
        let nonPrintingPermissions: [PDFAccessPermissions] = [
            .allowsDocumentAssembly,
            .allowsContentCopying,
            .allowsContentAccessibility,
            .allowsCommenting,
            .allowsFormFieldEntry
        ]
        let allowedBits = nonPrintingPermissions.reduce(UInt(0)) { $0 | $1.rawValue }
        XCTAssertTrue(document.write(to: protected, withOptions: [
            .userPasswordOption: "reader",
            .ownerPasswordOption: "owner",
            .accessPermissionsOption: NSNumber(value: allowedBits)
        ]))
        let workspace = PDFWorkspace()
        XCTAssertThrowsError(try workspace.open(protected))
        XCTAssertThrowsError(try workspace.open(protected, password: "wrong"))
        try workspace.open(protected, password: "reader")
        XCTAssertTrue(workspace.document?.string?.contains("Secret text") == true)
        XCTAssertTrue(workspace.isEncryptedSource)
        XCTAssertFalse(workspace.allowsPrinting)
        XCTAssertTrue(workspace.allowsCopying)
        XCTAssertTrue(workspace.allowsCommenting)
        XCTAssertFalse(workspace.allowsDocumentAssembly)
        try workspace.addPopupNote("private", author: nil, page: 0, bounds: CGRect(x: 20, y: 20, width: 20, height: 20), color: .yellow)
        workspace.undo()
        XCTAssertFalse(workspace.allowsPrinting); XCTAssertTrue(workspace.allowsCopying)
        XCTAssertTrue(workspace.allowsCommenting); XCTAssertFalse(workspace.allowsDocumentAssembly)
        workspace.redo()
        XCTAssertFalse(workspace.allowsPrinting); XCTAssertTrue(workspace.allowsCopying)
        XCTAssertTrue(workspace.allowsCommenting); XCTAssertFalse(workspace.allowsDocumentAssembly)

        let plaintext = directory.appendingPathComponent("must-not-be-plaintext.pdf")
        XCTAssertThrowsError(try workspace.saveCopy(to: plaintext)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied)
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: plaintext.path))

        let encrypted = directory.appendingPathComponent("encrypted-copy.pdf")
        let revision = workspace.revision
        let currentDocument = workspace.document
        XCTAssertThrowsError(try workspace.saveEncryptedCopy(to: encrypted, userPassword: "new-reader", ownerPassword: "new-owner")) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied)
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: encrypted.path))
        XCTAssertEqual(workspace.revision, revision)
        XCTAssertTrue(workspace.document === currentDocument)
        XCTAssertThrowsError(try workspace.saveEncryptedCopy(to: protected, userPassword: "x", ownerPassword: "y"))
        XCTAssertThrowsError(try workspace.saveEncryptedCopy(to: directory.appendingPathComponent("bad.pdf"), userPassword: "", ownerPassword: "y"))
    }

    func testUnencryptedSourceCanCreatePasswordProtectedCopy() throws {
        let plain = try makePDF("encrypt-plain.pdf", text: "Protect me")
        let workspace = PDFWorkspace(); try workspace.open(plain)
        XCTAssertFalse(workspace.isEncryptedSource)
        let encrypted = directory.appendingPathComponent("new-encrypted.pdf")
        try workspace.saveEncryptedCopy(to: encrypted, userPassword: "reader", ownerPassword: "owner")
        let locked = try XCTUnwrap(PDFDocument(url: encrypted))
        XCTAssertTrue(locked.isLocked)
        XCTAssertFalse(locked.unlock(withPassword: "wrong"))
        XCTAssertTrue(locked.unlock(withPassword: "reader"))
        XCTAssertTrue(locked.string?.contains("Protect me") == true)
    }

    func testOwnerOnlyEncryptedSourceCannotUseOrdinarySaveCopy() throws {
        let plain = try makePDF("owner-plain.pdf", text: "Owner protected")
        let protected = directory.appendingPathComponent("owner-only.pdf")
        let document = try XCTUnwrap(PDFDocument(url: plain))
        XCTAssertTrue(document.write(to: protected, withOptions: [.ownerPasswordOption: "owner-only"]))
        let reopened = try XCTUnwrap(PDFDocument(url: protected))
        XCTAssertTrue(reopened.isEncrypted)
        XCTAssertFalse(reopened.isLocked, "PDFs with an empty user password unlock automatically")

        let workspace = PDFWorkspace(); try workspace.open(protected)
        XCTAssertTrue(workspace.isEncryptedSource)
        XCTAssertFalse(workspace.allowsPrinting)
        XCTAssertFalse(workspace.allowsDocumentAssembly)
        let output = directory.appendingPathComponent("owner-only-plaintext.pdf")
        XCTAssertThrowsError(try workspace.saveCopy(to: output)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied)
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
        let encryptedOutput = directory.appendingPathComponent("owner-only-reencrypted.pdf")
        let revision = workspace.revision
        XCTAssertThrowsError(try workspace.saveEncryptedCopy(to: encryptedOutput, userPassword: "new-reader", ownerPassword: "new-owner")) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied)
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: encryptedOutput.path))
        XCTAssertEqual(workspace.revision, revision)
    }

    func testOrdinarySaveCopyHonorsSourceCopyPermission() throws {
        let plain = try makePDF("copy-source.pdf", text: "No copy")
        let protected = directory.appendingPathComponent("copy-restricted.pdf")
        let document = try XCTUnwrap(PDFDocument(url: plain))
        let allowed: [PDFAccessPermissions] = [
            .allowsLowQualityPrinting,
            .allowsHighQualityPrinting,
            .allowsDocumentAssembly,
            .allowsContentAccessibility,
            .allowsCommenting,
            .allowsFormFieldEntry
        ]
        XCTAssertTrue(document.write(to: protected, withOptions: [
            .ownerPasswordOption: "owner-only",
            .accessPermissionsOption: NSNumber(value: allowed.reduce(UInt(0)) { $0 | $1.rawValue })
        ]))
        let workspace = PDFWorkspace(); try workspace.open(protected)
        XCTAssertTrue(workspace.isEncryptedSource)
        XCTAssertFalse(workspace.allowsPrinting)
        XCTAssertFalse(workspace.allowsDocumentAssembly)
        let output = directory.appendingPathComponent("forbidden-copy.pdf")
        XCTAssertThrowsError(try workspace.saveCopy(to: output)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied)
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
    }

    private func makePDF(_ name: String, text: String) throws -> URL {
        let url = directory.appendingPathComponent(name)
        var box = CGRect(x: 0, y: 0, width: 612, height: 792)
        guard let consumer = CGDataConsumer(url: url as CFURL), let context = CGContext(consumer: consumer, mediaBox: &box, nil) else { throw PDFWorkspaceError.writeFailed }
        context.beginPDFPage(nil)
        NSGraphicsContext.saveGraphicsState(); NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: false)
        (text as NSString).draw(at: CGPoint(x: 72, y: 700), withAttributes: [.font: NSFont.systemFont(ofSize: 18)])
        NSGraphicsContext.restoreGraphicsState(); context.endPDFPage(); context.closePDF()
        return url
    }

    private func makeOutlinedPDF() throws -> URL {
        let first = try makePDF("one.pdf", text: "One")
        let second = try makePDF("two.pdf", text: "Two")
        let document = try XCTUnwrap(PDFDocument(url: first)); document.insert(try XCTUnwrap(PDFDocument(url: second)?.page(at: 0)), at: 1)
        let root = PDFOutline(); let existing = PDFOutline(); existing.label = "Existing"; existing.destination = PDFDestination(page: try XCTUnwrap(document.page(at: 0)), at: .zero); existing.destination?.zoom = 1.75
        let nested = PDFOutline(); nested.label = "Nested"; nested.destination = PDFDestination(page: try XCTUnwrap(document.page(at: 1)), at: .zero); nested.destination?.zoom = 2.25; existing.insertChild(nested, at: 0); root.insertChild(existing, at: 0); document.outlineRoot = root
        let output = directory.appendingPathComponent("outlined.pdf"); XCTAssertTrue(document.write(to: output)); return output
    }

    private func makeCatalogPDF() throws -> URL {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles 8 0 R >> /PageLabels << /Nums [0 << /P (A-) /S /D /St 1 >>] >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            "<< /Length 44 >>\nstream\nBT /F1 18 Tf 72 700 Td (Catalog) Tj ET\nendstream",
            "<< /Type /EmbeddedFile /Length 18 >>\nstream\ncatalog attachment\nendstream",
            "<< /Type /Filespec /F (generated.txt) /UF (generated.txt) /EF << /F 6 0 R >> >>",
            "<< /Names [(generated.txt) 7 0 R] >>"
        ]
        var data = Data("%PDF-1.7\n%\u{00e2}\u{00e3}\u{00cf}\u{00d3}\n".utf8)
        var offsets = [0]
        for (index, object) in objects.enumerated() {
            offsets.append(data.count)
            data.append(Data("\(index + 1) 0 obj\n\(object)\nendobj\n".utf8))
        }
        let xref = data.count
        data.append(Data("xref\n0 \(objects.count + 1)\n0000000000 65535 f \n".utf8))
        for offset in offsets.dropFirst() { data.append(Data(String(format: "%010d 00000 n \n", offset).utf8)) }
        data.append(Data("trailer\n<< /Size \(objects.count + 1) /Root 1 0 R >>\nstartxref\n\(xref)\n%%EOF\n".utf8))
        let output = directory.appendingPathComponent("catalog-source.pdf")
        try data.write(to: output)
        return output
    }

    private func catalogFeatures(at url: URL) throws -> (pageLabelPrefix: String?, embeddedFileName: String?, embeddedFileContents: Data?) {
        guard let document = CGPDFDocument(url as CFURL), let catalog = document.catalog else { throw PDFWorkspaceError.unreadableDocument }
        var pageLabels: CGPDFDictionaryRef?; CGPDFDictionaryGetDictionary(catalog, "PageLabels", &pageLabels)
        var nums: CGPDFArrayRef?; if let pageLabels { CGPDFDictionaryGetArray(pageLabels, "Nums", &nums) }
        var labelDictionary: CGPDFDictionaryRef?; if let nums { CGPDFArrayGetDictionary(nums, 1, &labelDictionary) }
        var prefix: CGPDFStringRef?; if let labelDictionary { CGPDFDictionaryGetString(labelDictionary, "P", &prefix) }

        var names: CGPDFDictionaryRef?; CGPDFDictionaryGetDictionary(catalog, "Names", &names)
        var embeddedFiles: CGPDFDictionaryRef?; if let names { CGPDFDictionaryGetDictionary(names, "EmbeddedFiles", &embeddedFiles) }
        var nameArray: CGPDFArrayRef?; if let embeddedFiles { CGPDFDictionaryGetArray(embeddedFiles, "Names", &nameArray) }
        var name: CGPDFStringRef?; if let nameArray { CGPDFArrayGetString(nameArray, 0, &name) }
        var fileSpec: CGPDFDictionaryRef?; if let nameArray { CGPDFArrayGetDictionary(nameArray, 1, &fileSpec) }
        var ef: CGPDFDictionaryRef?; if let fileSpec { CGPDFDictionaryGetDictionary(fileSpec, "EF", &ef) }
        var stream: CGPDFStreamRef?; if let ef { CGPDFDictionaryGetStream(ef, "F", &stream) }
        var format = CGPDFDataFormat.raw
        let contents = stream.flatMap { CGPDFStreamCopyData($0, &format) as Data? }
        func string(_ value: CGPDFStringRef?) -> String? {
            guard let value, let bytes = CGPDFStringGetBytePtr(value) else { return nil }
            return String(bytes: UnsafeBufferPointer(start: bytes, count: CGPDFStringGetLength(value)), encoding: .utf8)
        }
        return (string(prefix), string(name), contents)
    }
}
