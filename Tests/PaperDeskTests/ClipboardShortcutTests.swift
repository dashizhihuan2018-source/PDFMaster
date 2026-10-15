import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor final class ClipboardShortcutTests: XCTestCase {
    func testPDFPasteIsDisabledWithoutAnObjectClipboardHandler() {
        let view = InkPDFView()
        XCTAssertFalse(view.validateMenuItem(NSMenuItem(title: "粘贴", action: #selector(NSText.paste(_:)), keyEquivalent: "v")))
    }

    func testPageSurfacesRespondToStandardClipboardActions() {
        for view in [PageOverviewCollectionView() as NSView, ReaderPageThumbnailView()] {
            for action in [#selector(NSText.copy(_:)), #selector(NSText.cut(_:)), #selector(NSText.paste(_:)), #selector(NSText.selectAll(_:))] {
                XCTAssertTrue(view.responds(to: action), "\(type(of: view)) must handle \(action)")
            }
        }
    }

    func testSharedTypedClipboardExchangesPagesWithoutStaleFallback() throws {
        let pb = NSPasteboard.withUniqueName(); defer { pb.releaseGlobally() }
        let first = DocumentClipboard(pasteboard: pb), second = DocumentClipboard(pasteboard: pb)
        XCTAssertTrue(first.write(Data([1, 2, 3]), type: DocumentClipboard.pagesType))
        XCTAssertEqual(second.pages, Data([1, 2, 3]))
        XCTAssertNil(second.annotation)
        pb.clearContents(); pb.setString("new text", forType: .string)
        XCTAssertNil(first.pages)
    }

    func testTextHistoryOwnsUndoWithoutFallingBackToDocument() {
        let editor = NSTextView(); editor.allowsUndo = true
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 300, height: 200), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = editor; window.makeFirstResponder(editor)
        XCTAssertEqual(TextHistoryRouting.canPerform(in: window, redo: false), false)
        XCTAssertTrue(TextHistoryRouting.perform(in: window, redo: false))
        editor.insertText("hello", replacementRange: NSRange(location: 0, length: 0))
        editor.breakUndoCoalescing()
        XCTAssertEqual(TextHistoryRouting.canPerform(in: window, redo: false), true)
        XCTAssertTrue(TextHistoryRouting.perform(in: window, redo: false))
        XCTAssertEqual(editor.string, "")
        XCTAssertTrue(TextHistoryRouting.perform(in: window, redo: true))
        XCTAssertEqual(editor.string, "hello")
        window.contentView = nil
    }

    func testCrossTabPageCopyPasteSelectAllCutAndUndo() throws {
        let pb = NSPasteboard.withUniqueName(); defer { pb.releaseGlobally() }
        let source = WorkbenchController(); _ = source.view
        let target = WorkbenchController(); _ = target.view
        for c in [source, target] {
            c.clipboard = DocumentClipboard(pasteboard: pb)
            try c.workspace.createBlank(); try c.workspace.insertBlank(at: 1)
            c.refreshDocument(); c.enterMode(.pages)
        }
        source.pageOverview.restoreSelection(IndexSet([0, 1]))
        source.pageOverview.collectionView.copy(nil)
        target.pageOverview.collectionView.paste(nil)
        XCTAssertEqual(target.workspace.document?.pageCount, 4)
        XCTAssertEqual(target.pageOverview.selectedPages, IndexSet([1, 2]))
        target.undo(); XCTAssertEqual(target.workspace.document?.pageCount, 2)
        target.redo(); XCTAssertEqual(target.workspace.document?.pageCount, 4)
        target.pageOverview.collectionView.selectAll(nil)
        XCTAssertEqual(target.pageOverview.selectedPages, IndexSet(integersIn: 0..<4))
        XCTAssertFalse(target.canPageClipboard(.cut, overview: true), "Cannot cut all pages")
        target.pageOverview.restoreSelection(IndexSet(integer: 1))
        target.pageOverview.collectionView.cut(nil)
        XCTAssertEqual(target.workspace.document?.pageCount, 3)
        target.undo(); XCTAssertEqual(target.workspace.document?.pageCount, 4)
    }

    func testAnnotationObjectCopyPasteAndCutUseOneUndoStep() throws {
        let pb = NSPasteboard.withUniqueName(); defer { pb.releaseGlobally() }
        let c = WorkbenchController(); _ = c.view; c.clipboard = DocumentClipboard(pasteboard: pb)
        try c.workspace.createBlank()
        try c.workspace.addShape(.rectangle, page: 0, bounds: CGRect(x: 30, y: 60, width: 90, height: 60), style: .init(strokeColor: .red, fillColor: nil, lineWidth: 2))
        c.refreshDocument(); c.pdfView.onAnnotationSelection?(0, 0)
        c.pdfView.copy(nil); XCTAssertNotNil(c.clipboard.annotation)
        c.pdfView.paste(nil); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 2)
        c.undo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
        c.pdfView.onAnnotationSelection?(0, 0)
        c.pdfView.cut(nil); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
        c.undo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 1)
    }

    func testPlainTextPastesAsEditableAnnotation() throws {
        let pb = NSPasteboard.withUniqueName(); defer { pb.releaseGlobally() }
        pb.setString("快捷键粘贴测试", forType: .string)
        let c = WorkbenchController(); _ = c.view; c.clipboard = DocumentClipboard(pasteboard: pb)
        try c.workspace.createBlank(); c.refreshDocument()
        c.pdfView.paste(nil)
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.type, "FreeText")
        XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.first?.contents, "快捷键粘贴测试")
        c.undo(); XCTAssertEqual(c.workspace.document?.page(at: 0)?.annotations.count, 0)
    }

    func testExpertAnnotationBindingsAndStandardTextTargets() throws {
        for (action, key) in [(#selector(WorkbenchController.addRectangle), "r"), (#selector(WorkbenchController.highlight), "h"), (#selector(WorkbenchController.toggleInk(_:)), "p"), (#selector(WorkbenchController.selectEraser), "e")] {
            let item = NSMenuItem()
            ExpertShortcuts.apply(to: item, action: action)
            XCTAssertEqual(item.keyEquivalent, key)
            XCTAssertEqual(item.keyEquivalentModifierMask, [.command, .control])
        }
        let menu = NSMenu(); DocumentMenuSelectors.addTextEditingItems(to: menu)
        XCTAssertTrue(menu.items.allSatisfy { $0.target == nil })
        XCTAssertEqual(menu.items.first { $0.action == #selector(NSTextView.pasteAsPlainText(_:)) }?.keyEquivalent, "V")
    }

    func testNativeThumbnailSelectionReplacesRatherThanAccumulates() throws {
        let c = WorkbenchController(); _ = c.view
        try c.workspace.createBlank(); try c.workspace.insertBlank(at: 1); try c.workspace.insertBlank(at: 2)
        c.refreshDocument()
        let window = NSWindow(contentRect: CGRect(x: 0, y: 0, width: 1280, height: 820), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentViewController = c; c.view.layoutSubtreeIfNeeded()
        c.readerSidebar.thumbnailView.layoutSubtreeIfNeeded()
        c.readerSidebar.thumbnailView.selectPages(IndexSet([0, 1]))
        c.readerSidebar.thumbnailView.selectPages(IndexSet(integer: 2))
        let document = try XCTUnwrap(c.pdfView.document)
        XCTAssertEqual(c.readerSidebar.thumbnailView.selectedPages?.map { document.index(for: $0) }, [2])
        window.contentViewController = nil
    }
}
