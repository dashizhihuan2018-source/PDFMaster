import AppKit
import PDFKit
import XCTest
import PaperCore
@testable import PaperDesk

@MainActor final class ContentTextDialogTests: XCTestCase {
    private var root: URL { URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent() }

    func testCompactFangSongChineseAppliesUndoRedoAndSavedCopyReopens() async throws {
        let original = "合同管理办法，原文文字测试", replacement = "合同管理办法，修改文字测试"
        let hex = original.utf16.map { String(format: "%04X", $0) }.joined()
        let stream = "BT /Original 12 Tf 80 592 Td <\(hex)> Tj ET\n"
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /Original 5 0 R >> >> /Contents 4 0 R >>",
            "<< /Length \(stream.utf8.count) >>\nstream\n\(stream)endstream",
            "<< /Type /Font /Subtype /Type0 /BaseFont /FangSong_GB2312 /Encoding /UniGB-UTF16-H /DescendantFonts [6 0 R] >>",
            "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /FangSong_GB2312 /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 5 >> /FontDescriptor 7 0 R /DW 1000 >>",
            "<< /Type /FontDescriptor /FontName /FangSong_GB2312 /FontBBox [0 -120 1000 880] /Flags 4 /ItalicAngle 0 /Ascent 880 /Descent -120 /StemV 80 >>"
        ]
        var pdf = "%PDF-1.7\n", offsets = [0]
        for (i, object) in objects.enumerated() { offsets.append(pdf.utf8.count); pdf += "\(i + 1) 0 obj\n\(object)\nendobj\n" }
        let xref = pdf.utf8.count
        pdf += "xref\n0 8\n0000000000 65535 f \n" + offsets.dropFirst().map { String(format: "%010d 00000 n \n", $0) }.joined()
        pdf += "trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n\(xref)\n%%EOF\n"
        let temp = FileManager.default.temporaryDirectory.appendingPathComponent("TextFangSong-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: temp, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: temp) }
        let source = temp.appendingPathComponent("source.pdf"), snapshot = temp.appendingPathComponent("snapshot.pdf"), saved = temp.appendingPathComponent("saved.pdf")
        let originalData = Data(pdf.utf8); try originalData.write(to: source)
        let c = WorkbenchController(); _ = c.view; try c.workspace.open(source); c.refreshDocument()
        let input = try c.workspace.engineInput(); try input.data.write(to: snapshot)
        let engine = ContentEditEngine(script: root.appendingPathComponent("Resources/ContentEdit/engine.py"))
        let span = try XCTUnwrap(engine.inspect(snapshot, page: 0, cancelled: { false }).spans.first { $0.text == original })
        XCTAssertEqual(span.font, "FangSong_GB2312")
        var request = ContentEditRequest(operation: "replace-text", page: 0); request.id = span.id; request.text = replacement
        try await c.applyContentEdit(request, input: input, engine: engine)
        XCTAssertTrue(c.pdfView.document?.page(at: 0)?.string?.contains(replacement) == true)
        c.undo(); XCTAssertTrue(c.pdfView.document?.page(at: 0)?.string?.contains(original) == true)
        c.redo(); XCTAssertTrue(c.pdfView.document?.page(at: 0)?.string?.contains(replacement) == true)
        try c.workspace.saveCopy(to: saved)
        let reopened = try XCTUnwrap(PDFDocument(url: saved))
        XCTAssertTrue(reopened.page(at: 0)?.string?.contains(replacement) == true)
        XCTAssertFalse(reopened.page(at: 0)?.string?.contains(original) == true)
        XCTAssertEqual(try Data(contentsOf: source), originalData)
    }

    func testFailedApplyShowsReasonAndKeepsDraftForRetry() async throws {
        _ = NSApplication.shared
        let fixture = root.appendingPathComponent("work/parallel/content-edit/evidence/content-edit-qa.pdf")
        let engine = ContentEditEngine(script: root.appendingPathComponent("Resources/ContentEdit/engine.py"))
        let c = WorkbenchController(); _ = c.view
        try c.workspace.open(fixture); c.refreshDocument()
        let input = try c.workspace.engineInput()
        let temp = FileManager.default.temporaryDirectory.appendingPathComponent("TextDialog-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: temp, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: temp) }
        let source = temp.appendingPathComponent("input.pdf"); try input.data.write(to: source)
        let span = try XCTUnwrap(engine.inspect(source, page: 0, cancelled: { false }).spans.first { $0.text == "Original target" })
        let window = NSWindow(contentRect: NSRect(x: -4000, y: 0, width: 1100, height: 800), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentViewController = c; window.orderFront(nil)
        defer {
            if let sheet = window.attachedSheet { window.endSheet(sheet, returnCode: .alertSecondButtonReturn) }
            window.orderOut(nil); window.contentViewController = nil
        }
        c.presentTextReplacement(span, page: 0, input: input, engine: engine, window: window)
        let draft = String(repeating: "TOO LONG ", count: 25)
        let field = try XCTUnwrap(Self.fields(window.attachedSheet?.contentView).first)
        field.stringValue = draft
        let apply = try XCTUnwrap(Self.views(window.attachedSheet?.contentView).compactMap { $0 as? NSButton }.first { $0.title == "应用" })
        apply.performClick(nil)
        // Exercise the real sheet completion, async engine, and retry dialog.
        for _ in 0..<250 {
            if Self.fields(window.attachedSheet?.contentView).contains(where: { $0.stringValue == draft }),
               Self.views(window.attachedSheet?.contentView).compactMap({ $0 as? NSTextField }).contains(where: { $0.stringValue.contains("无法应用") }) { break }
            try await Task.sleep(for: .milliseconds(40))
        }
        XCTAssertTrue(Self.fields(window.attachedSheet?.contentView).contains { $0.stringValue == draft }, "A failed edit must keep the user's draft available for correction")
        XCTAssertTrue(Self.views(window.attachedSheet?.contentView).compactMap { $0 as? NSTextField }.contains { $0.stringValue.contains("无法应用") }, "The failure reason must be in the dialog, not only the hidden status bar")
        XCTAssertFalse(c.workspace.isDirty); XCTAssertFalse(c.workspace.canUndo)
        XCTAssertFalse(c.isLocalToolRunning)
        guard let retryField = Self.fields(window.attachedSheet?.contentView).first,
              let retry = Self.views(window.attachedSheet?.contentView).compactMap({ $0 as? NSButton }).first(where: { $0.title == "应用" }) else { return }
        retryField.stringValue = "Edited target"; retry.performClick(nil)
        for _ in 0..<250 {
            if c.workspace.document?.page(at: 0)?.string?.contains("Edited target") == true, !c.isLocalToolRunning, window.attachedSheet == nil { break }
            try await Task.sleep(for: .milliseconds(40))
        }
        XCTAssertTrue(c.workspace.document?.page(at: 0)?.string?.contains("Edited target") == true)
        XCTAssertNil(window.attachedSheet)
        c.undo(); XCTAssertTrue(c.workspace.document?.page(at: 0)?.string?.contains("Original target") == true)
    }

    private static func fields(_ view: NSView?) -> [NSTextField] { views(view).compactMap { $0 as? NSTextField }.filter { $0.isEditable } }
    private static func views(_ view: NSView?) -> [NSView] {
        guard let view else { return [] }; return [view] + view.subviews.flatMap { views($0) }
    }
}
