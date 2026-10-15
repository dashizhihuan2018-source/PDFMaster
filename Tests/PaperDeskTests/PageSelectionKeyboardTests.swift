import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor
final class PageSelectionKeyboardTests: XCTestCase {
    func testOverviewDeleteAndForwardDeleteInvokeSelectionCallback() throws {
        let view = PageOverviewCollectionView()
        let window = host(view)
        defer { window.orderOut(nil) }
        XCTAssertTrue(window.makeFirstResponder(view))
        var calls = 0
        view.onDeleteSelection = { calls += 1 }

        view.keyDown(with: key(51, window: window))
        view.keyDown(with: key(117, window: window, modifiers: [.function]))

        XCTAssertEqual(calls, 2, "Both backward Delete and Fn/forward Delete must delete the selected pages")
    }

    func testSidebarDeleteAndForwardDeleteInvokeSelectionCallback() throws {
        let view = ReaderPageThumbnailView()
        let window = host(view)
        defer { window.orderOut(nil) }
        XCTAssertTrue(window.makeFirstResponder(view))
        var calls = 0
        view.onDeleteSelection = { calls += 1 }

        view.keyDown(with: key(51, window: window))
        view.keyDown(with: key(117, window: window, modifiers: [.function]))

        XCTAssertEqual(calls, 2)
    }

    func testModifiedDeleteDoesNotBecomePageDeletion() throws {
        let view = PageOverviewCollectionView()
        let window = host(view)
        defer { window.orderOut(nil) }
        XCTAssertTrue(window.makeFirstResponder(view))
        var calls = 0
        view.onDeleteSelection = { calls += 1 }

        for modifier: NSEvent.ModifierFlags in [.command, .option, .control, .shift] {
            view.keyDown(with: key(51, window: window, modifiers: modifier))
        }
        view.keyDown(with: key(124, window: window))
        view.keyDown(with: key(6, window: window, modifiers: .command))

        XCTAssertEqual(calls, 0, "Existing arrow, undo, and modified-delete handling must not become page deletion")
    }

    func testTextEditorFocusPreventsEitherPageViewFromDeletingPages() throws {
        for view in [PageOverviewCollectionView(), ReaderPageThumbnailView()] as [NSView] {
            let window = host(view)
            defer { window.orderOut(nil) }
            var calls = 0
            if let overview = view as? PageOverviewCollectionView { overview.onDeleteSelection = { calls += 1 } }
            if let thumbnails = view as? ReaderPageThumbnailView { thumbnails.onDeleteSelection = { calls += 1 } }
            let field = NSTextField(string: "abc")
            field.frame = NSRect(x: 10, y: 10, width: 140, height: 24)
            view.addSubview(field)
            XCTAssertTrue(window.makeFirstResponder(field))
            let editor = try XCTUnwrap(window.firstResponder as? NSTextView)
            editor.setSelectedRange(NSRange(location: 3, length: 0))

            window.sendEvent(key(51, window: window))

            XCTAssertEqual(field.stringValue, "ab")
            XCTAssertEqual(calls, 0)
            XCTAssertTrue(window.firstResponder === editor)
        }
    }

    func testSidebarRoutesSelectedPresentationPageIndexesToOwner() throws {
        let pdfView = PDFView(frame: NSRect(x: 0, y: 0, width: 400, height: 500))
        let document = PDFDocument()
        for _ in 0..<3 { document.insert(PDFPage(), at: document.pageCount) }
        let session = ReaderSession(pdfView: pdfView)
        session.setDocument(document, allowsCopying: true)
        let sidebar = ReaderSidebarController(session: session)
        let window = host(sidebar.view)
        defer { window.orderOut(nil) }
        pdfView.go(to: try XCTUnwrap(document.page(at: 1)))
        XCTAssertTrue(window.makeFirstResponder(sidebar.thumbnailView))
        var deleted: IndexSet?
        sidebar.onDeletePages = { deleted = $0 }

        sidebar.thumbnailView.keyDown(with: key(51, window: window))

        XCTAssertEqual(deleted, IndexSet(integer: 1), "Delete must use the sidebar selection's zero-based page indexes")
    }

    func testSidebarDeleteRouterAcceptsPDFKitsInternalCollectionResponder() throws {
        let pdfView = PDFView(frame: NSRect(x: 0, y: 0, width: 400, height: 500))
        let document = PDFDocument()
        for _ in 0..<3 { document.insert(PDFPage(), at: document.pageCount) }
        let session = ReaderSession(pdfView: pdfView)
        session.setDocument(document, allowsCopying: true)
        let sidebar = ReaderSidebarController(session: session)
        let window = host(sidebar.view)
        defer { window.orderOut(nil) }
        sidebar.view.layoutSubtreeIfNeeded()
        pdfView.go(to: try XCTUnwrap(document.page(at: 1)))
        let thumbnail = sidebar.thumbnailView
        func findCollection(_ view: NSView) -> NSCollectionView? {
            if let collection = view as? NSCollectionView { return collection }
            return view.subviews.lazy.compactMap { findCollection($0) }.first
        }
        let collection = try XCTUnwrap(findCollection(thumbnail))
        collection.selectionIndexPaths = [IndexPath(item: 0, section: 0), IndexPath(item: 2, section: 0)]
        XCTAssertTrue(window.makeFirstResponder(collection))
        var deleted: IndexSet?
        sidebar.onDeletePages = { deleted = $0 }

        // NSWindow.sendEvent bypasses app-local event monitors. Exercise the
        // actual pre-dispatch router with PDFKit's real internal first responder;
        // OS/AppKit monitor delivery requires separate app E2E verification.
        XCTAssertTrue(thumbnail.handlePageDeletionKey(key(51, window: window)))

        XCTAssertEqual(deleted, IndexSet([0, 2]), "PDFKit's internal collection selection must retain both selected pages")
    }

    func testSidebarRouterDoesNotConsumeOtherKeysOrAnotherWindowsEvents() throws {
        let view = ReaderPageThumbnailView()
        let window = host(view)
        let other = host(NSView())
        defer { window.orderOut(nil); other.orderOut(nil) }
        XCTAssertTrue(window.makeFirstResponder(view))
        var calls = 0
        view.onDeleteSelection = { calls += 1 }

        XCTAssertFalse(view.handlePageDeletionKey(key(51, window: other)))
        for modifier: NSEvent.ModifierFlags in [.command, .option, .control, .shift] {
            XCTAssertFalse(view.handlePageDeletionKey(key(51, window: window, modifiers: modifier)))
        }
        XCTAssertFalse(view.handlePageDeletionKey(key(124, window: window)))
        XCTAssertFalse(view.handlePageDeletionKey(key(6, window: window, modifiers: .command)))
        view.isHidden = true
        XCTAssertFalse(view.handlePageDeletionKey(key(51, window: window)))
        XCTAssertEqual(calls, 0)
    }

    func testSidebarPreDispatchRouterRejectsFieldEditorInsideItsView() throws {
        let view = ReaderPageThumbnailView()
        let window = host(view)
        defer { window.orderOut(nil) }
        let field = NSTextField(string: "abc")
        field.frame = NSRect(x: 10, y: 10, width: 140, height: 24)
        view.addSubview(field)
        XCTAssertTrue(window.makeFirstResponder(field))
        var calls = 0
        view.onDeleteSelection = { calls += 1 }

        XCTAssertFalse(view.handlePageDeletionKey(key(51, window: window)))
        XCTAssertEqual(calls, 0)
        XCTAssertTrue(window.firstResponder is NSTextView)
    }

    func testPageSelectionSurfacesDoNotMoveWindowFromDocumentBackground() {
        for view in [PageOverviewCollectionView(), ReaderPageThumbnailView()] as [NSView] {
            let window = host(view)
            defer { window.orderOut(nil) }
            window.isMovableByWindowBackground = true

            XCTAssertFalse(view.mouseDownCanMoveWindow, "\(type(of: view)) page selection must not initiate window dragging")
            XCTAssertTrue(window.isMovableByWindowBackground, "Keep the app's custom titlebar dragging enabled")
        }
    }

    func testSidebarRouterRejectsHiddenAncestorAndFocusOutsideThumbnail() throws {
        let root = NSView()
        let view = ReaderPageThumbnailView(frame: NSRect(x: 0, y: 0, width: 200, height: 300))
        root.addSubview(view)
        let window = host(root)
        defer { window.orderOut(nil) }
        var calls = 0
        view.onDeleteSelection = { calls += 1 }
        XCTAssertTrue(window.makeFirstResponder(view))
        root.isHidden = true
        XCTAssertFalse(view.handlePageDeletionKey(key(51, window: window)))
        root.isHidden = false
        let field = NSSearchField(string: "query")
        root.addSubview(field)
        XCTAssertTrue(window.makeFirstResponder(field))
        XCTAssertFalse(view.handlePageDeletionKey(key(51, window: window)))

        XCTAssertEqual(calls, 0)
    }

    func testSidebarRouterTracksWindowChangesAndDetachedViewCannotHandleOldEvents() throws {
        let view = ReaderPageThumbnailView()
        let first = host(view)
        let second = host(NSView())
        defer { first.orderOut(nil); second.orderOut(nil) }
        var calls = 0
        view.onDeleteSelection = { calls += 1 }
        XCTAssertTrue(first.makeFirstResponder(view))
        XCTAssertTrue(view.handlePageDeletionKey(key(51, window: first)))

        first.contentView = NSView()
        XCTAssertNil(view.window)
        XCTAssertFalse(view.handlePageDeletionKey(key(51, window: first)))
        second.contentView = view
        XCTAssertTrue(second.makeFirstResponder(view))
        XCTAssertFalse(view.handlePageDeletionKey(key(51, window: first)))
        XCTAssertTrue(view.handlePageDeletionKey(key(117, window: second)))
        XCTAssertEqual(calls, 2)
    }

    func testSidebarLocalMonitorDoesNotRetainRemovedView() {
        weak var removed: ReaderPageThumbnailView?
        autoreleasepool {
            var view: ReaderPageThumbnailView? = ReaderPageThumbnailView()
            let window = host(view!)
            defer { window.orderOut(nil) }
            removed = view
            view?.onDeleteSelection = { }
            window.contentView = NSView()
            view = nil
        }
        XCTAssertNil(removed, "The App-local monitor must not keep a removed thumbnail sidebar alive")
    }

    func testDirectGridKeyDownDoesNotDeleteWithNestedFieldEditorFocus() throws {
        let grid = PageOverviewCollectionView()
        let nested = NSView(frame: NSRect(x: 0, y: 0, width: 250, height: 100))
        let field = NSTextField(string: "nested text")
        field.frame = NSRect(x: 10, y: 10, width: 140, height: 24)
        nested.addSubview(field); grid.addSubview(nested)
        let window = host(grid)
        defer { window.orderOut(nil) }
        XCTAssertTrue(window.makeFirstResponder(field))
        let editor = try XCTUnwrap(window.firstResponder as? NSTextView)
        var calls = 0
        grid.onDeleteSelection = { calls += 1 }

        grid.keyDown(with: key(51, window: window))
        grid.keyDown(with: key(117, window: window, modifiers: .function))

        XCTAssertEqual(calls, 0)
        XCTAssertTrue(window.firstResponder === editor)
    }

    private func host(_ view: NSView) -> NSWindow {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 480, height: 400),
                              styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = view
        return window
    }

    private func key(_ code: UInt16, window: NSWindow, modifiers: NSEvent.ModifierFlags = []) -> NSEvent {
        let characters = code == 51 ? "\u{7f}" : code == 117 ? "\u{F728}" : code == 124 ? "\u{F703}" : "z"
        return NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: modifiers,
                               timestamp: 0, windowNumber: window.windowNumber, context: nil,
                               characters: characters, charactersIgnoringModifiers: characters,
                               isARepeat: false, keyCode: code)!
    }
}
