import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor final class InlineTextEditingTests: XCTestCase {
    private func fixture() throws -> (WorkbenchController, NSWindow) {
        _ = NSApplication.shared
        let c = WorkbenchController(); _ = c.view
        let w = NSWindow(contentRect: CGRect(x: 0, y: 0, width: 1100, height: 800), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        w.contentViewController = c
        try c.workspace.createBlank(); c.refreshDocument(); c.enterMode(.annotation)
        c.view.layoutSubtreeIfNeeded(); c.pdfView.layoutDocumentView()
        return (c, w)
    }
    private func draft(_ c: WorkbenchController, text: String, start: CGPoint = CGPoint(x: 100, y: 600)) throws -> PDFInlineTextSession {
        c.addFreeText()
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        c.pdfView.beginGesture(on: page, at: start); c.pdfView.finishGesture()
        let session = try XCTUnwrap(c.pdfView.inlineTextSession)
        session.textView.insertText(text, replacementRange: NSRange(location: NSNotFound, length: 0))
        return session
    }
    func testToolArmsWithoutSheetAndClickFocusesNativeEditorWithoutMutatingPDF() throws {
        let (c, w) = try fixture(); defer { w.contentViewController = nil }
        let revision = c.workspace.revision
        let session = try draft(c, text: "自由输入 Chinese 123")
        XCTAssertNil(w.attachedSheet)
        XCTAssertTrue(w.firstResponder === session.textView)
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        XCTAssertTrue(session.textView.allowsUndo)
        XCTAssertFalse(session.textView.isRichText)
        XCTAssertTrue(c.pdfView.finishInlineTextEditing(commit: false))
        XCTAssertEqual(c.workspace.revision, revision)
    }
    func testMultilineCommitsOnceAndUndoRedoSaveReopenPreserveAllText() throws {
        let (c, w) = try fixture(); defer { w.contentViewController = nil }
        let text = "第一行：自由输入\n第二行：回车换行\nThird line 123"
        let revision = c.workspace.revision
        _ = try draft(c, text: text)
        XCTAssertTrue(c.pdfView.finishInlineTextEditing())
        XCTAssertEqual(c.workspace.revision, revision + 1)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.contents, text)
        XCTAssertTrue(c.pdfView.freeTextInputEnabled)
        c.undo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        c.redo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.contents, text)
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("InlineText-\(UUID()).pdf")
        defer { try? FileManager.default.removeItem(at: file) }
        try c.workspace.saveCopy(to: file)
        let saved = try XCTUnwrap(PDFDocument(url: file)?.page(at: 0)?.annotations.first)
        XCTAssertEqual(saved.contents, text)
        XCTAssertEqual(saved.font?.pointSize ?? 0, 14, accuracy: 0.1)
        XCTAssertTrue(saved.shouldDisplay)
    }
    func testTypingUndoDoesNotUndoDocumentAndEscapeDiscardsOnlyDraft() throws {
        let (c, w) = try fixture(); defer { w.contentViewController = nil }
        let session = try draft(c, text: "abc")
        session.textView.breakUndoCoalescing()
        session.textView.insertText("def", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(TextHistoryRouting.perform(in: w, redo: false))
        XCTAssertNotEqual(session.textView.string, "abcdef")
        XCTAssertTrue(TextHistoryRouting.perform(in: w, redo: true))
        XCTAssertEqual(session.textView.string, "abcdef")
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        session.textView.cancelOperation(nil)
        XCTAssertNil(c.pdfView.inlineTextSession)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
    }

    func testLargeChineseDraftReservesPDFKitLineGridAndStandardBoldFont() throws {
        let (c, w) = try fixture(); defer { w.contentViewController = nil }
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        let font = try XCTUnwrap(NSFont(name: "Helvetica-Bold", size: 24))
        let session = PDFInlineTextSession(pdfView: c.pdfView, page: page,
            bounds: CGRect(x: 100, y: 500, width: 240, height: 36), text: "", font: font, color: .black,
            commit: { text, bounds, font, color in
                try c.workspace.addText(text, page: 0, bounds: bounds, font: font, color: color)
            }, failure: { XCTFail($0.localizedDescription) })
        c.pdfView.inlineTextSession = session
        session.textView.insertText("像画布一样自由输入文字\n第二行：回车继续写\nPDF Master · 页面内编辑",
                                   replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertGreaterThanOrEqual(session.pageBounds.height, 144)
        XCTAssertTrue(session.finish())
        let saved = try XCTUnwrap(c.workspace.document?.page(at: 0)?.annotations.first)
        XCTAssertEqual(saved.font?.fontName, "Helvetica-Bold")
        XCTAssertEqual(saved.font?.pointSize, 24)
        XCTAssertGreaterThanOrEqual(saved.bounds.height, 144)
    }
    func testEmptyDraftAddsNothingAndExistingTextReeditsInPlaceWithoutDuplicates() throws {
        let (c, w) = try fixture(); defer { w.contentViewController = nil }
        _ = try draft(c, text: "")
        let revision = c.workspace.revision
        XCTAssertTrue(c.pdfView.finishInlineTextEditing())
        XCTAssertEqual(c.workspace.revision, revision)
        _ = try draft(c, text: "原文字")
        XCTAssertTrue(c.pdfView.finishInlineTextEditing())
        c.editFreeTextInline(page: 0, annotationIndex: 0)
        let session = try XCTUnwrap(c.pdfView.inlineTextSession)
        session.textView.selectAll(nil); session.textView.insertText("新文字\n继续编辑", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(c.pdfView.finishInlineTextEditing())
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations[0].contents, "新文字\n继续编辑")
        c.undo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations[0].contents, "原文字")
    }
    func testStaleWorkspaceKeepsDraftInsteadOfWritingToAnotherDocument() throws {
        let (c, w) = try fixture(); defer { w.contentViewController = nil }
        let session = try draft(c, text: "不要写错文档")
        try c.workspace.insertBlank(at: 1)
        XCTAssertFalse(c.pdfView.finishInlineTextEditing())
        XCTAssertTrue(c.pdfView.inlineTextSession === session)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        XCTAssertTrue(c.interactionStatusText.contains("过期"))
        XCTAssertTrue(c.pdfView.finishInlineTextEditing(commit: false))
    }
    func testPageOverflowPreservesDraftAndNoHistoryEntry() throws {
        let (c, w) = try fixture(); defer { w.contentViewController = nil }
        let revision = c.workspace.revision
        let session = try draft(c, text: Array(repeating: "不能静默裁掉文字", count: 100).joined(separator: "\n"), start: CGPoint(x: 100, y: 50))
        XCTAssertFalse(c.pdfView.finishInlineTextEditing())
        XCTAssertEqual(c.workspace.revision, revision)
        XCTAssertTrue(c.pdfView.inlineTextSession === session)
        XCTAssertTrue(c.interactionStatusText.contains("超出页面"))
        _ = c.pdfView.finishInlineTextEditing(commit: false)
    }
    func testToolSwitchAndSaveCommitDraftBeforeAction() throws {
        let (c, w) = try fixture(); defer { w.contentViewController = nil }
        _ = try draft(c, text: "切换前提交")
        c.enterMode(.editing)
        XCTAssertNil(c.pdfView.inlineTextSession)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations[0].contents, "切换前提交")
        _ = try draft(c, text: "保存前提交", start: CGPoint(x: 100, y: 450))
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("InlineSave-\(UUID()).pdf")
        defer { try? FileManager.default.removeItem(at: file) }
        XCTAssertTrue(c.saveDocumentInteractively(chooseDestination: { file }))
        XCTAssertEqual(PDFDocument(url: file)?.page(at: 0)?.annotations.count, 2)
    }
    func testEditingTextToolAlsoCreatesTextInBlankAreaWithoutContentEngine() throws {
        let (c, w) = try fixture(); defer { w.contentViewController = nil }
        c.enterMode(.editing); c.selectContentTool("content-text")
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        c.pdfView.beginGesture(on: page, at: CGPoint(x: 90, y: 600)); c.pdfView.finishGesture()
        XCTAssertNotNil(c.pdfView.inlineTextSession)
        _ = c.pdfView.finishInlineTextEditing(commit: false)
    }
    func testEditorTracksZoomScrollAndAllPageRotationsWithinCrop() throws {
        for rotation in [0, 90, 180, 270] {
            let (c, w) = try fixture(); defer { w.contentViewController = nil }
            if rotation != 0 { try c.workspace.rotate(page: 0, degrees: rotation); c.refreshDocument() }
            c.pdfView.scaleFactor = 1.2; c.pdfView.layoutDocumentView()
            let session = try draft(c, text: "位置保持", start: CGPoint(x: 180, y: 420))
            c.pdfView.scaleFactor = 1.7; c.pdfView.layoutDocumentView(); session.reposition()
            let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
            let expected = c.pdfView.convert(session.pageBounds, from: page)
            XCTAssertEqual(session.container.frame.minX, expected.minX, accuracy: 0.1)
            XCTAssertEqual(session.container.frame.maxY, expected.maxY, accuracy: 0.1)
            XCTAssertTrue(c.pdfView.finishInlineTextEditing(), "rotation \(rotation)")
            let annotation = try XCTUnwrap(c.workspace.document?.page(at: 0)?.annotations.first)
            XCTAssertTrue(page.bounds(for: .cropBox).contains(annotation.bounds))
            XCTAssertEqual(annotation.contents, "位置保持")
        }
    }
    func testNativeMarkedTextStaysInDraftAndEscapeDoesNotDiscardDocument() throws {
        let (c, w) = try fixture(); defer { w.contentViewController = nil }
        let session = try draft(c, text: "已输入")
        session.textView.setMarkedText("拼音候选", selectedRange: NSRange(location: 4, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(session.textView.hasMarkedText())
        session.textView.unmarkText()
        XCTAssertTrue(c.pdfView.finishInlineTextEditing())
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.contents, "已输入拼音候选")
    }
    func testDeleteAllExistingTextRemovesOnlyThatTextAndUndoRestoresIt() throws {
        let (c, w) = try fixture(); defer { w.contentViewController = nil }
        _ = try draft(c, text: "保留原内容"); XCTAssertTrue(c.pdfView.finishInlineTextEditing())
        c.editFreeTextInline(page: 0, annotationIndex: 0)
        let session = try XCTUnwrap(c.pdfView.inlineTextSession)
        session.textView.selectAll(nil); session.textView.delete(nil)
        XCTAssertTrue(c.pdfView.finishInlineTextEditing())
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        c.undo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.contents, "保留原内容")
    }
    func testOriginalContentNowUsesInlineEditorAndCancelLeavesSourceUnchanged() throws {
        let (c, w) = try fixture(); defer { w.contentViewController = nil }
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let source = root.appendingPathComponent("work/parallel/content-edit/evidence/content-edit-qa.pdf")
        let original = try Data(contentsOf: source)
        try c.workspace.open(source); c.refreshDocument(); c.enterMode(.editing)
        let engine = ContentEditEngine(script: root.appendingPathComponent("Resources/ContentEdit/engine.py"))
        let span = try XCTUnwrap(engine.inspect(source, page: 0, cancelled: { false }).spans.first { $0.text == "Original target" })
        let input = try c.workspace.engineInput()
        c.presentInlineTextReplacement(span, page: 0, input: input, engine: engine, window: w)
        XCTAssertNil(w.attachedSheet)
        let session = try XCTUnwrap(c.pdfView.inlineTextSession)
        XCTAssertEqual(session.textView.string, "Original target")
        session.textView.selectAll(nil); session.textView.insertText("修改\n多行", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertFalse(c.pdfView.finishInlineTextEditing(), "Original single-span constraints must retain the draft, never silently add an overlay")
        XCTAssertTrue(c.pdfView.inlineTextSession === session)
        XCTAssertTrue(c.pdfView.finishInlineTextEditing(commit: false))
        XCTAssertEqual(try Data(contentsOf: source), original)
        XCTAssertTrue(c.workspace.document?.page(at: 0)?.string?.contains("Original target") == true)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, PDFDocument(data: original)?.page(at: 0)?.annotations.count)
    }
    func testCroppingNonzeroOriginAndDraggingSpecifiesWidthAtPointer() throws {
        let (c, w) = try fixture(); defer { w.contentViewController = nil }
        let crop = CGRect(x: 37, y: 61, width: 400, height: 600)
        try c.workspace.cropPage(at: 0, to: crop); c.refreshDocument()
        c.addFreeText(); c.pdfView.scaleFactor = 1.4; c.pdfView.layoutDocumentView()
        let page = try XCTUnwrap(c.pdfView.document?.page(at: 0))
        c.pdfView.beginGesture(on: page, at: CGPoint(x: 90, y: 500))
        c.pdfView.updateGesture(to: CGPoint(x: 270, y: 420)); c.pdfView.finishGesture()
        let session = try XCTUnwrap(c.pdfView.inlineTextSession)
        XCTAssertEqual(session.pageBounds.width, 180, accuracy: 0.1)
        XCTAssertTrue(crop.contains(session.pageBounds))
        session.textView.insertText("裁剪页坐标", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(c.pdfView.finishInlineTextEditing())
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.bounds.minX ?? 0, 90, accuracy: 0.1)
    }
    func testTabCloseFlushesDraftBeforeDirtyDecision() throws {
        let recents = FileManager.default.temporaryDirectory.appendingPathComponent("InlineClose-\(UUID()).json")
        defer { try? FileManager.default.removeItem(at: recents) }
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: recents)); _ = shell.view
        let tab = shell.createBlankDocument()
        let c = try XCTUnwrap(shell.activeWorkbench)
        c.beginFreeTextInput(from: CGPoint(x: 100, y: 600), to: CGPoint(x: 100, y: 600), page: 0)
        let session = try XCTUnwrap(c.pdfView.inlineTextSession)
        session.textView.insertText("关闭前不能丢字", replacementRange: NSRange(location: NSNotFound, length: 0))
        var asked = false
        XCTAssertFalse(shell.close(tabID: tab.id, dirtyDecision: { asked = true; return false }))
        XCTAssertTrue(asked)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.contents, "关闭前不能丢字")
    }
}
