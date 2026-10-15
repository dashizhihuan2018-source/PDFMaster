import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

final class DocumentToolControlsTests: XCTestCase {
    func testGeometryAndStyleValidationRejectsInvalidValuesWithoutGuessing() throws {
        let media = CGRect(x: 0, y: 0, width: 612, height: 792)
        XCTAssertEqual(
            try ToolInputValidation.rectangle(x: "20", y: "30", width: "100", height: "80", within: media),
            CGRect(x: 20, y: 30, width: 100, height: 80)
        )
        XCTAssertThrowsError(try ToolInputValidation.rectangle(x: "nan", y: "30", width: "100", height: "80", within: media))
        XCTAssertThrowsError(try ToolInputValidation.rectangle(x: "600", y: "30", width: "100", height: "80", within: media))
        XCTAssertThrowsError(try ToolInputValidation.style(stroke: .black, fill: nil, width: "0"))
        XCTAssertEqual(try ToolInputValidation.style(stroke: .red, fill: .yellow, width: "2.5").lineWidth, 2.5)
    }

    @MainActor func testNewDocumentActionsAreDisabledWithoutADocument() {
        let controller = WorkbenchController()
        _ = controller.view
        let selectors: [Selector] = [
            #selector(WorkbenchController.saveFlattenedCopy),
            #selector(WorkbenchController.savePasswordProtectedCopy),
            #selector(WorkbenchController.exportPlainText),
            #selector(WorkbenchController.exportAnnotationSummaryText),
            #selector(WorkbenchController.addPopupNote),
            #selector(WorkbenchController.addRectangle),
            #selector(WorkbenchController.editSelectedAnnotation),
            #selector(WorkbenchController.deleteSelectedAnnotation),
            #selector(WorkbenchController.duplicateCurrentPage),
            #selector(WorkbenchController.cropCurrentPage),
            #selector(WorkbenchController.showOutline),
            #selector(WorkbenchController.addCurrentPageBookmark)
        ]

        for selector in selectors {
            let item = NSMenuItem(title: "test", action: selector, keyEquivalent: "")
            XCTAssertFalse(controller.validateMenuItem(item), "\(selector) must be disabled without a document")
        }
    }

    func testKnownPasswordAdmissionCancellationAndWrongPasswordDoNotReplaceWorkspace() throws {
        let source = try makeEncryptedPDF(userPassword: "reader", ownerPassword: "owner")
        var replacements = 0
        var dirtyChecks = 0

        let cancelled = try DocumentAdmission.admit(
            url: source,
            passwordPrompt: { nil },
            shouldReplace: { dirtyChecks += 1; return true },
            replace: { _ in replacements += 1 }
        )
        XCTAssertFalse(cancelled)
        XCTAssertEqual(dirtyChecks, 0)
        XCTAssertEqual(replacements, 0)

        XCTAssertThrowsError(
            try DocumentAdmission.admit(
                url: source,
                passwordPrompt: { "wrong" },
                shouldReplace: { dirtyChecks += 1; return true },
                replace: { _ in replacements += 1 }
            )
        ) { XCTAssertEqual($0 as? PDFWorkspaceError, .lockedDocument) }
        XCTAssertEqual(dirtyChecks, 0)
        XCTAssertEqual(replacements, 0)
    }

    func testTextExportRejectsSourceAndAliases() throws {
        let directory = temporaryDirectory()
        let source = directory.appendingPathComponent("source.pdf")
        try Data("source".utf8).write(to: source)
        let symlink = directory.appendingPathComponent("alias.txt")
        try FileManager.default.createSymbolicLink(at: symlink, withDestinationURL: source)
        let hardlink = directory.appendingPathComponent("hardlink.txt")
        try FileManager.default.linkItem(at: source, to: hardlink)

        let protection = try XCTUnwrap(SourceProtection.capture(source))
        XCTAssertThrowsError(try SafeTextExport.write("summary", to: source, protection: protection)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .cannotOverwriteSource)
        }
        XCTAssertThrowsError(try SafeTextExport.write("summary", to: symlink, protection: protection)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .cannotOverwriteSource)
        }
        XCTAssertThrowsError(try SafeTextExport.write("summary", to: hardlink, protection: protection)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .cannotOverwriteSource)
        }

        let output = directory.appendingPathComponent("summary.txt")
        try SafeTextExport.write("summary", to: output, protection: protection)
        XCTAssertEqual(try String(contentsOf: output, encoding: .utf8), "summary")
    }

    func testCapturedSourceProtectionRejectsOriginalHardlinkAfterSourcePathReplacement() throws {
        let directory = temporaryDirectory()
        let source = directory.appendingPathComponent("source.pdf")
        let original = Data("original bytes".utf8); try original.write(to: source)
        let survivingHardlink = directory.appendingPathComponent("original-hardlink.txt")
        try FileManager.default.linkItem(at: source, to: survivingHardlink)
        let protection = try XCTUnwrap(SourceProtection.capture(source))

        try Data("replacement inode".utf8).write(to: source, options: .atomic)
        XCTAssertEqual(try Data(contentsOf: survivingHardlink), original)
        XCTAssertThrowsError(try SafeTextExport.write("summary", to: survivingHardlink, protection: protection)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .cannotOverwriteSource)
        }
        XCTAssertEqual(try Data(contentsOf: survivingHardlink), original)
    }

    @MainActor func testPasswordProtectedCopyUsesCapturedEncryptionStateAfterSourcePathReplacement() throws {
        let encrypted = try makeEncryptedPDF(userPassword: "reader", ownerPassword: "owner")
        let workspace = PDFWorkspace(); try workspace.open(encrypted, password: "reader")
        XCTAssertTrue(workspace.isEncryptedSource)
        XCTAssertFalse(PasswordProtectedCopyPolicy.isAvailable(in: workspace))

        let plain = encrypted.deletingLastPathComponent().appendingPathComponent("plain.pdf")
        let plainDocument = PDFDocument(); plainDocument.insert(try XCTUnwrap(PDFPage(image: whiteImage())), at: 0)
        XCTAssertTrue(plainDocument.write(to: plain))
        try Data(contentsOf: plain).write(to: encrypted, options: .atomic)

        XCTAssertFalse(PDFDocument(url: encrypted)?.isEncrypted == true, "fixture path now contains a plain PDF")
        XCTAssertTrue(workspace.isEncryptedSource, "workspace must retain admission-time encryption state")
        XCTAssertFalse(PasswordProtectedCopyPolicy.isAvailable(in: workspace))
    }

    @MainActor func testSelectedAnnotationIsInvalidatedAfterWorkspaceRevisionChanges() throws {
        let workspace = PDFWorkspace()
        try workspace.createBlank()
        try workspace.addPopupNote("first", author: nil, page: 0, bounds: CGRect(x: 20, y: 20, width: 30, height: 30), color: .yellow)
        let tracker = AnnotationSelectionTracker()
        tracker.select(page: 0, index: 0, in: workspace)
        XCTAssertNotNil(tracker.current(in: workspace))

        try workspace.addPopupNote("second", author: nil, page: 0, bounds: CGRect(x: 60, y: 20, width: 30, height: 30), color: .yellow)
        XCTAssertNil(tracker.current(in: workspace))
        XCTAssertNil(tracker.selection)
    }

    @MainActor func testSelectedAnnotationIsInvalidatedWhenReplacementHasSameRevisionPageAndIndex() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        try workspace.addPopupNote("old", author: nil, page: 0, bounds: CGRect(x: 20, y: 20, width: 30, height: 30), color: .yellow)
        let tracker = AnnotationSelectionTracker(); tracker.select(page: 0, index: 0, in: workspace)
        let oldRevision = workspace.revision

        try workspace.createBlank()
        try workspace.addPopupNote("new", author: nil, page: 0, bounds: CGRect(x: 20, y: 20, width: 30, height: 30), color: .yellow)
        XCTAssertEqual(workspace.revision, oldRevision)
        XCTAssertNil(tracker.current(in: workspace))
    }

    @MainActor func testNewDocumentAndEscapeClearTransientInteractionModes() throws {
        let controller = WorkbenchController(); _ = controller.view
        let inkMenuItem = NSMenuItem(title: "手绘模式", action: #selector(WorkbenchController.toggleInk(_:)), keyEquivalent: "")
        controller.registerInkMenuItem(inkMenuItem)
        try controller.workspace.createBlank(); controller.refreshDocument()
        controller.toggleInk(inkMenuItem)
        controller.pdfView.placementEnabled = true
        controller.setInteractionStatus("旧批注放置提示")
        XCTAssertEqual(inkMenuItem.state, .on)
        controller.newDocument()
        XCTAssertFalse(controller.pdfView.inkEnabled)
        XCTAssertFalse(controller.pdfView.placementEnabled)
        XCTAssertEqual(inkMenuItem.state, .off)
        XCTAssertEqual(controller.interactionStatusText, DocumentStatusPresentation.message(hasDocument: true, isEncryptedSource: false))

        let plain = temporaryDirectory().appendingPathComponent("replacement.pdf")
        let replacement = PDFDocument(); replacement.insert(try XCTUnwrap(PDFPage(image: whiteImage())), at: 0)
        XCTAssertTrue(replacement.write(to: plain))
        controller.pdfView.inkEnabled = true
        controller.pdfView.placementEnabled = true
        inkMenuItem.state = .on
        controller.setInteractionStatus("另一个旧提示")
        XCTAssertTrue(controller.open(url: plain))
        XCTAssertFalse(controller.pdfView.inkEnabled)
        XCTAssertFalse(controller.pdfView.placementEnabled)
        XCTAssertEqual(inkMenuItem.state, .off)
        XCTAssertEqual(controller.interactionStatusText, DocumentStatusPresentation.message(hasDocument: true, isEncryptedSource: false))

        controller.pdfView.placementEnabled = true
        controller.setInteractionStatus("待取消放置")
        XCTAssertTrue(controller.pdfView.cancelPlacementForEscape())
        XCTAssertFalse(controller.pdfView.placementEnabled)
        XCTAssertEqual(controller.interactionStatusText, DocumentStatusPresentation.message(hasDocument: true, isEncryptedSource: false))
    }

    @MainActor func testEncryptedDocumentStatusAndPrintReasonDescribeConservativeBuildLimitation() throws {
        let encrypted = try makeEncryptedPDF(userPassword: "reader", ownerPassword: "owner")
        let controller = WorkbenchController(); _ = controller.view
        try controller.workspace.open(encrypted, password: "reader")
        controller.refreshDocument()

        XCTAssertTrue(controller.interactionStatusText.contains("当前版本"))
        XCTAssertTrue(controller.interactionStatusText.contains("打印"))
        XCTAssertTrue(controller.interactionStatusText.contains("页面装配"))
        XCTAssertTrue(controller.interactionStatusToolTip?.contains("无法可靠判定") == true)

        let encryptedReason = DocumentOperationMessage.printBlockReason(
            hasDocument: true,
            isEncryptedSource: true,
            allowsPrinting: false
        )
        XCTAssertTrue(encryptedReason?.contains("当前版本") == true)
        XCTAssertFalse(encryptedReason?.contains("原始权限不允许") == true)

        let permissionReason = DocumentOperationMessage.printBlockReason(
            hasDocument: true,
            isEncryptedSource: false,
            allowsPrinting: false
        )
        XCTAssertTrue(permissionReason?.contains("原始权限不允许") == true)
        XCTAssertNil(DocumentOperationMessage.printBlockReason(hasDocument: true, isEncryptedSource: false, allowsPrinting: true))
    }

    @MainActor func testEncryptedExtractCancellationCreatesNoPlaintextAndChangesNoState() throws {
        let encrypted = try makeEncryptedPDF(userPassword: "reader", ownerPassword: "owner")
        let controller = WorkbenchController(); _ = controller.view
        try controller.workspace.open(encrypted, password: "reader"); controller.refreshDocument()
        let revision = controller.workspace.revision
        let document = controller.workspace.document
        let destination = encrypted.deletingLastPathComponent().appendingPathComponent("cancelled-extract.pdf")
        var confirmations = 0

        XCTAssertFalse(controller.performExtractPages(IndexSet(integer: 0), to: destination) {
            confirmations += 1
            return false
        })
        XCTAssertEqual(confirmations, 1)
        XCTAssertFalse(FileManager.default.fileExists(atPath: destination.path))
        XCTAssertEqual(controller.workspace.revision, revision)
        XCTAssertTrue(controller.workspace.document === document)
        XCTAssertTrue(ExtractPagesProtection.warning.contains("明文"))
        XCTAssertTrue(ExtractPagesProtection.warning.contains("保护"))
    }

    func testOutlineRemovalDisclosureDoesNotPromiseNestedChildrenArePreserved() {
        XCTAssertTrue(OutlineEditDisclosure.information(removing: true).contains("子项"))
        XCTAssertTrue(OutlineEditDisclosure.information(removing: true).contains("删除"))
        XCTAssertTrue(OutlineEditDisclosure.information(removing: true).contains("撤销"))
        XCTAssertTrue(OutlineEditDisclosure.information(removing: false).contains("保留"))
    }

    @MainActor func testStandardCopyCutSelectorsPreserveTextResponderRoutingAndGatePDFView() {
        XCTAssertEqual(DocumentMenuSelectors.copy, #selector(NSText.copy(_:)))
        XCTAssertEqual(DocumentMenuSelectors.cut, #selector(NSText.cut(_:)))
        let textView = NSTextView()
        XCTAssertTrue(textView.responds(to: DocumentMenuSelectors.copy))
        XCTAssertTrue(textView.responds(to: DocumentMenuSelectors.cut))

        let pdfView = InkPDFView()
        pdfView.allowsCopying = false
        XCTAssertFalse(pdfView.isPDFActionAllowed(DocumentMenuSelectors.copy, hasSelection: true))
        pdfView.allowsCopying = true
        XCTAssertTrue(pdfView.isPDFActionAllowed(DocumentMenuSelectors.copy, hasSelection: true))
        XCTAssertFalse(pdfView.isPDFActionAllowed(DocumentMenuSelectors.copy, hasSelection: false))
        XCTAssertFalse(pdfView.isPDFActionAllowed(DocumentMenuSelectors.cut, hasSelection: true))
    }
    @MainActor func testStandardTextMenuIncludesPasteAndSelectAllWithoutDocumentTarget() throws {
        let menu = NSMenu(title: "编辑")
        DocumentMenuSelectors.addTextEditingItems(to: menu)
        for (action, key) in [(#selector(NSText.paste(_:)), "v"), (#selector(NSText.selectAll(_:)), "a")] {
            let item = try XCTUnwrap(menu.items.first { $0.action == action },
                                     "Native text shortcuts must be installed, including on a canvas-only tab")
            XCTAssertNil(item.target)
            XCTAssertEqual(item.keyEquivalent, key)
            XCTAssertEqual(item.keyEquivalentModifierMask, .command)
            XCTAssertTrue(NSTextView().responds(to: action))
        }
    }

    @MainActor func testDuplicateAndCropControllerSeamsCallCoreTransactions() throws {
        let controller = WorkbenchController()
        _ = controller.view
        try controller.workspace.createBlank()
        controller.refreshDocument()

        XCTAssertTrue(controller.duplicateCurrentPage())
        XCTAssertEqual(controller.workspace.document?.pageCount, 2)
        XCTAssertTrue(controller.applyCrop(CGRect(x: 20, y: 30, width: 300, height: 400)))
        XCTAssertEqual(controller.workspace.document?.page(at: 0)?.bounds(for: .cropBox), CGRect(x: 20, y: 30, width: 300, height: 400))
    }

    func testOutlineProjectionIncludesNestedNodesAndTopLevelEditIdentity() throws {
        let document = try makeOutlinedPDF()
        let projection = OutlineProjection.make(from: document)
        XCTAssertEqual(projection.map(\.label), ["Chapter", "Nested topic", "Second chapter"])
        XCTAssertEqual(projection.map(\.pageNumber), [1, 2, 2])
        XCTAssertEqual(projection.map(\.depth), [0, 1, 0])
        XCTAssertEqual(projection.map(\.topLevelIndex), [0, nil, 1])
    }

    private func makeEncryptedPDF(userPassword: String, ownerPassword: String) throws -> URL {
        let document = PDFDocument()
        document.insert(try XCTUnwrap(PDFPage(image: whiteImage())), at: 0)
        let url = temporaryDirectory().appendingPathComponent("locked.pdf")
        XCTAssertTrue(document.write(to: url, withOptions: [
            PDFDocumentWriteOption.userPasswordOption: userPassword,
            PDFDocumentWriteOption.ownerPasswordOption: ownerPassword
        ]))
        return url
    }

    private func makeOutlinedPDF() throws -> PDFDocument {
        let document = PDFDocument()
        document.insert(try XCTUnwrap(PDFPage(image: whiteImage())), at: 0)
        document.insert(try XCTUnwrap(PDFPage(image: whiteImage())), at: 1)
        let root = PDFOutline()
        let chapter = PDFOutline(); chapter.label = "Chapter"; chapter.destination = PDFDestination(page: try XCTUnwrap(document.page(at: 0)), at: .zero)
        let nested = PDFOutline(); nested.label = "Nested topic"; nested.destination = PDFDestination(page: try XCTUnwrap(document.page(at: 1)), at: .zero)
        chapter.insertChild(nested, at: 0)
        let second = PDFOutline(); second.label = "Second chapter"; second.destination = PDFDestination(page: try XCTUnwrap(document.page(at: 1)), at: .zero)
        root.insertChild(chapter, at: 0); root.insertChild(second, at: 1); document.outlineRoot = root
        return document
    }

    private func whiteImage() -> NSImage {
        let image = NSImage(size: NSSize(width: 612, height: 792))
        image.lockFocus(); NSColor.white.setFill(); NSRect(x: 0, y: 0, width: 612, height: 792).fill(); image.unlockFocus()
        return image
    }

    private func temporaryDirectory() -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("PaperDeskUI-\(UUID().uuidString)", isDirectory: true)
        try! FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }
}
