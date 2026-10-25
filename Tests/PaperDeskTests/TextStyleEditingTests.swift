import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor final class TextStyleEditingTests: XCTestCase {
    private var root: URL { URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent() }

    private func fixture() throws -> (WorkbenchController, NSWindow, PDFInlineTextSession) {
        _ = NSApplication.shared
        let controller = WorkbenchController(); _ = controller.view
        let window = NSWindow(contentRect: CGRect(x: 0, y: 0, width: 1100, height: 800), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.contentViewController = controller
        try controller.workspace.createBlank(); controller.refreshDocument()
        controller.beginFreeTextInput(from: CGPoint(x: 100, y: 650), to: CGPoint(x: 100, y: 650), page: 0)
        return (controller, window, try XCTUnwrap(controller.pdfView.inlineTextSession))
    }

    private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    private func combo(_ label: String, in controller: WorkbenchController) throws -> NSComboBox {
        try XCTUnwrap(descendants(controller.view).compactMap { $0 as? NSComboBox }.first { $0.accessibilityLabel() == label }, "A native editable combo must expose \(label)")
    }
    private func apply(_ value: String, to combo: NSComboBox) {
        combo.stringValue = value
        if let action = combo.action { NSApp.sendAction(action, to: combo.target, from: combo) }
    }

    func testTypedFontFamilyAndArbitraryIntegerSizePreviewAndPersist() throws {
        let (controller, window, session) = try fixture(); defer { window.contentViewController = nil }
        session.textView.insertText("Editable font\n可编辑字体", replacementRange: NSRange(location: NSNotFound, length: 0))
        let family = try combo("文字字体", in: controller)
        let size = try combo("文字字号", in: controller)
        XCTAssertTrue(family.isEditable)
        XCTAssertTrue(size.isEditable)
        apply("Times", to: family)
        apply("17", to: size)
        let typingFont = try XCTUnwrap(session.textView.typingAttributes[.font] as? NSFont)
        XCTAssertEqual(typingFont.familyName, "Times")
        XCTAssertEqual(typingFont.pointSize, 17 * controller.pdfView.scaleFactor, accuracy: 0.02)
        XCTAssertTrue(controller.pdfView.finishInlineTextEditing())
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("TextStyle-\(UUID()).pdf")
        defer { try? FileManager.default.removeItem(at: file) }
        try controller.workspace.saveCopy(to: file)
        let saved = try XCTUnwrap(PDFDocument(url: file)?.page(at: 0)?.annotations.first)
        XCTAssertEqual(saved.font?.familyName, "Times")
        XCTAssertEqual(saved.font?.pointSize ?? 0, 17, accuracy: 0.02)
    }

    func testChineseFontFamiliesPersistTheChosenFaceRatherThanHelvetica() throws {
        for name in ["STSongti-SC-Bold", "PingFangSC-Regular"] {
            let font = try XCTUnwrap(NSFont(name: name, size: 27))
            let workspace = PDFWorkspace(); try workspace.createBlank()
            try workspace.addText("中文字体清晰度 Font 123", page: 0,
                bounds: CGRect(x: 100, y: 500, width: 400, height: 90), font: font,
                color: NSColor(srgbRed: 0.2, green: 0.4, blue: 0.8, alpha: 1))
            let saved = try XCTUnwrap(PDFDocument(data: try XCTUnwrap(workspace.document?.dataRepresentation()))?.page(at: 0)?.annotations.first)
            XCTAssertEqual(saved.font?.fontName, font.fontName)
            XCTAssertEqual(saved.font?.pointSize, 27)
        }
    }

    func testOriginalSingleSpanEditorEnablesFontFamilySizeAndColorControls() throws {
        let (controller, window, _) = try fixture(); defer { window.contentViewController = nil }
        _ = controller.pdfView.finishInlineTextEditing(commit: false)
        let source = root.appendingPathComponent("work/parallel/content-edit/evidence/content-edit-qa.pdf")
        let engine = ContentEditEngine(script: root.appendingPathComponent("Resources/ContentEdit/engine.py"))
        try controller.workspace.open(source); controller.refreshDocument(); controller.enterMode(.editing)
        let span = try XCTUnwrap(engine.inspect(source, page: 0, cancelled: { false }).spans.first { $0.text == "Original target" })
        controller.presentInlineTextReplacement(span, page: 0, input: try controller.workspace.engineInput(), engine: engine, window: window)
        let family = try combo("文字字体", in: controller)
        let size = try combo("文字字号", in: controller)
        let color = try XCTUnwrap(descendants(controller.view).compactMap { $0 as? NSColorWell }.first { $0.accessibilityLabel() == "文字颜色" })
        XCTAssertTrue(family.isEnabled)
        XCTAssertTrue(size.isEnabled)
        XCTAssertTrue(color.isEnabled)
        _ = controller.pdfView.finishInlineTextEditing(commit: false)
    }

    func testInvalidSizeKeepsDraftAndUnchangedDocumentUntilCorrected() throws {
        let (controller, window, session) = try fixture(); defer { window.contentViewController = nil }
        session.textView.string = "保留草稿"
        let size = try combo("文字字号", in: controller)
        for invalid in ["0", "513", "17.5", "nan", "inf", "not a size"] {
            apply(invalid, to: size)
            XCTAssertFalse(controller.pdfView.finishInlineTextEditing())
            XCTAssertTrue(controller.pdfView.inlineTextSession === session)
            XCTAssertFalse(controller.workspace.isDirty)
            XCTAssertEqual(session.textView.string, "保留草稿")
        }
        apply("19", to: size)
        XCTAssertTrue(controller.pdfView.finishInlineTextEditing())
        XCTAssertEqual(controller.workspace.document?.page(at: 0)?.annotations.first?.font?.pointSize ?? 0, 19, accuracy: 0.02)
    }

    func testExistingAnnotationFontChangePreservesBoldAndColorAcrossReopen() throws {
        let (controller, window, initial) = try fixture(); defer { window.contentViewController = nil }
        _ = initial.finish(commit: false)
        let bold = try XCTUnwrap(NSFont(name: "Helvetica-Bold", size: 18))
        try controller.workspace.addText("Editable style", page: 0, bounds: CGRect(x: 80, y: 600, width: 320, height: 80), font: bold)
        controller.refreshDocument(); controller.editFreeTextInline(page: 0, annotationIndex: 0)
        let session = try XCTUnwrap(controller.pdfView.inlineTextSession)
        apply("Times", to: try combo("文字字体", in: controller))
        XCTAssertTrue(NSFontManager.shared.traits(of: try XCTUnwrap(session.textView.font)).contains(.boldFontMask))
        XCTAssertEqual((session.textView.font?.pointSize ?? 0) / controller.pdfView.scaleFactor, 18, accuracy: 0.02)
        let color = try XCTUnwrap(descendants(controller.view).compactMap { $0 as? NSColorWell }.first)
        XCTAssertTrue(color.isContinuous)
        color.color = NSColor(srgbRed: 0.2, green: 0.4, blue: 0.8, alpha: 1)
        if let action = color.action { NSApp.sendAction(action, to: color.target, from: color) }
        XCTAssertEqual(try XCTUnwrap(session.textView.textColor?.usingColorSpace(.sRGB)).blueComponent, 0.8, accuracy: 0.01)
        XCTAssertTrue(session.finish())
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("TextStyle-existing-\(UUID()).pdf")
        defer { try? FileManager.default.removeItem(at: file) }
        try controller.workspace.saveCopy(to: file)
        let saved = try XCTUnwrap(PDFDocument(url: file)?.page(at: 0)?.annotations.first)
        XCTAssertTrue(NSFontManager.shared.traits(of: try XCTUnwrap(saved.font)).contains(.boldFontMask))
        XCTAssertEqual(saved.font?.familyName, "Times")
        XCTAssertEqual(saved.font?.pointSize ?? 0, 18, accuracy: 0.02)
        XCTAssertEqual(try XCTUnwrap(saved.fontColor?.usingColorSpace(.sRGB)).blueComponent, 0.8, accuracy: 0.01)
    }

    func testOriginalUnchangedTextCanCommitNewStyleIntoNativeContent() async throws {
        let (controller, window, initial) = try fixture(); defer { window.contentViewController = nil }
        _ = initial.finish(commit: false)
        let source = root.appendingPathComponent("work/parallel/content-edit/evidence/content-edit-qa.pdf")
        let original = try Data(contentsOf: source)
        let engine = ContentEditEngine(script: root.appendingPathComponent("Resources/ContentEdit/engine.py"))
        try controller.workspace.open(source); controller.refreshDocument(); controller.enterMode(.editing)
        let span = try XCTUnwrap(engine.inspect(source, page: 0, cancelled: { false }).spans.first { $0.text == "Original target" })
        controller.presentInlineTextReplacement(span, page: 0, input: try controller.workspace.engineInput(), engine: engine, window: window)
        apply("Times", to: try combo("文字字体", in: controller))
        apply("10.5", to: try combo("文字字号", in: controller))
        let color = try XCTUnwrap(descendants(controller.view).compactMap { $0 as? NSColorWell }.first)
        color.color = NSColor(srgbRed: 1, green: 0, blue: 0, alpha: 1)
        if let action = color.action { NSApp.sendAction(action, to: color.target, from: color) }
        XCTAssertTrue(controller.pdfView.finishInlineTextEditing())
        for _ in 0..<400 where controller.isContentEditPending { try await Task.sleep(for: .milliseconds(20)) }
        XCTAssertFalse(controller.isContentEditPending)
        XCTAssertTrue(controller.workspace.isDirty, "Changing style alone must write original page content: \(controller.interactionStatusText)")
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("TextStyle-original-\(UUID()).pdf")
        defer { try? FileManager.default.removeItem(at: file) }
        try controller.workspace.saveCopy(to: file)
        let result = try engine.inspect(file, page: 0, cancelled: { false })
        let edited = try XCTUnwrap(result.spans.first { $0.text == "Original target" })
        XCTAssertTrue(edited.font.contains("Times"))
        XCTAssertEqual(edited.size, 10.5, accuracy: 0.02)
        XCTAssertEqual(edited.color, 0xFF0000)
        XCTAssertTrue(result.spans.contains { $0.text == "Neighbor preserved" })
        XCTAssertFalse(PDFDocument(url: file)!.page(at: 0)!.annotations.contains { $0.type == "FreeText" || $0.type == "Square" })
        controller.undo()
        let undoFile = file.deletingLastPathComponent().appendingPathComponent("TextStyle-undo-\(UUID()).pdf")
        let redoFile = file.deletingLastPathComponent().appendingPathComponent("TextStyle-redo-\(UUID()).pdf")
        defer { try? FileManager.default.removeItem(at: undoFile); try? FileManager.default.removeItem(at: redoFile) }
        try controller.workspace.saveCopy(to: undoFile)
        let undone = try XCTUnwrap(engine.inspect(undoFile, page: 0, cancelled: { false }).spans.first { $0.text == span.text })
        XCTAssertEqual(undone.font, span.font)
        XCTAssertEqual(undone.size, span.size, accuracy: 0.02)
        XCTAssertEqual(undone.color, span.color)
        controller.redo()
        try controller.workspace.saveCopy(to: redoFile)
        let redone = try XCTUnwrap(engine.inspect(redoFile, page: 0, cancelled: { false }).spans.first { $0.text == span.text })
        XCTAssertTrue(redone.font.contains("Times"))
        XCTAssertEqual(redone.size, 10.5, accuracy: 0.02)
        XCTAssertEqual(redone.color, 0xFF0000)
        XCTAssertEqual(try Data(contentsOf: source), original)
    }
}
