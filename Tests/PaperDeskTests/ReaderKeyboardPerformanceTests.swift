import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor
final class ReaderKeyboardPerformanceTests: XCTestCase {
    private let arrowKeys: [(keyCode: UInt16, delta: Int)] = [
        (123, -1), // left
        (126, -1), // up
        (124, 1),  // right
        (125, 1)   // down
    ]

    func testAllFourArrowKeysTurnPagesWithoutMutatingDocument() throws {
        for arrow in arrowKeys {
            let controller = try makeWorkbench(pageCount: 3)
            let middle = try XCTUnwrap(controller.pdfView.document?.page(at: 1))
            controller.pdfView.go(to: middle)
            let revision = controller.workspace.revision

            controller.pdfView.keyDown(with: keyEvent(arrow.keyCode, window: controller.pdfView.window))

            XCTAssertEqual(currentPage(in: controller.pdfView), 1 + arrow.delta, "keyCode \(arrow.keyCode)")
            XCTAssertEqual(controller.workspace.revision, revision, "page navigation must be read-only")
        }
    }

    func testArrowNavigationClampsAtFirstAndLastPage() throws {
        let controller = try makeWorkbench(pageCount: 3)
        controller.pdfView.go(to: try XCTUnwrap(controller.pdfView.document?.page(at: 0)))
        controller.pdfView.keyDown(with: keyEvent(123, window: controller.pdfView.window))
        controller.pdfView.keyDown(with: keyEvent(126, window: controller.pdfView.window))
        XCTAssertEqual(currentPage(in: controller.pdfView), 0)

        controller.pdfView.go(to: try XCTUnwrap(controller.pdfView.document?.page(at: 2)))
        controller.pdfView.keyDown(with: keyEvent(124, window: controller.pdfView.window))
        controller.pdfView.keyDown(with: keyEvent(125, window: controller.pdfView.window))
        XCTAssertEqual(currentPage(in: controller.pdfView), 2)
    }

    func testSelectedShapeConsumesArrowForNudgeBeforePageNavigation() throws {
        let controller = try makeWorkbench(pageCount: 3)
        try controller.workspace.addShape(
            .rectangle,
            page: 1,
            bounds: CGRect(x: 100, y: 100, width: 80, height: 60),
            style: .default
        )
        controller.refreshDocument(preservePage: 1)
        controller.pdfView.selectShape(page: 1, index: 0)
        let revision = controller.workspace.revision

        controller.pdfView.keyDown(with: keyEvent(124, window: controller.pdfView.window))

        XCTAssertEqual(currentPage(in: controller.pdfView), 1)
        XCTAssertEqual(controller.workspace.revision, revision + 1)
        let moved = try XCTUnwrap(controller.workspace.document?.page(at: 1)?.annotations[0].bounds)
        XCTAssertEqual(moved.minX, 101, accuracy: 0.01)
    }

    func testSelectedInkConsumesArrowWithoutTurningPageOrMutatingDocument() throws {
        let controller = try makeWorkbench(pageCount: 3)
        try controller.workspace.addInk(
            points: [CGPoint(x: 100, y: 100), CGPoint(x: 140, y: 120)],
            page: 1, color: .black, width: 2, smoothed: false
        )
        controller.refreshDocument(preservePage: 1)
        controller.pdfView.selectInk(page: 1, index: 0)
        let revision = controller.workspace.revision

        controller.pdfView.keyDown(with: keyEvent(124, window: controller.pdfView.window))

        XCTAssertEqual(currentPage(in: controller.pdfView), 1)
        XCTAssertEqual(controller.workspace.revision, revision)
    }

    func testSelectedSingleArrowNudgePreservesEndpointStylesAndOneUndoRestoresBounds() throws {
        let controller = try makeWorkbench(pageCount: 3)
        try controller.workspace.addLine(
            from: CGPoint(x: 100, y: 100), to: CGPoint(x: 180, y: 160), page: 1,
            style: .default, endings: .arrow
        )
        controller.refreshDocument(preservePage: 1)
        controller.pdfView.selectShape(page: 1, index: 0)
        let original = try XCTUnwrap(controller.workspace.document?.page(at: 1)?.annotations[0])
        let originalBounds = original.bounds
        let originalStartStyle = original.startLineStyle
        let originalEndStyle = original.endLineStyle
        let revision = controller.workspace.revision

        controller.pdfView.keyDown(with: keyEvent(124, window: controller.pdfView.window))

        let moved = try XCTUnwrap(controller.workspace.document?.page(at: 1)?.annotations[0])
        XCTAssertEqual(controller.workspace.revision, revision + 1)
        XCTAssertEqual(moved.bounds.minX, originalBounds.minX + 1, accuracy: 0.01)
        XCTAssertEqual(moved.startLineStyle, originalStartStyle)
        XCTAssertEqual(moved.endLineStyle, originalEndStyle)

        controller.undo()
        let restored = try XCTUnwrap(controller.workspace.document?.page(at: 1)?.annotations[0])
        XCTAssertEqual(restored.bounds, originalBounds)
        XCTAssertEqual(restored.startLineStyle, originalStartStyle)
        XCTAssertEqual(restored.endLineStyle, originalEndStyle)
    }

    func testTextFieldFirstResponderKeepsArrowInsteadOfTurningPDFPage() throws {
        let controller = try makeWorkbench(pageCount: 3)
        let window = try XCTUnwrap(controller.pdfView.window)
        let field = NSTextField(string: "abc")
        field.frame = CGRect(x: 10, y: 10, width: 160, height: 24)
        controller.view.addSubview(field)
        XCTAssertTrue(window.makeFirstResponder(field))
        XCTAssertTrue(window.firstResponder is NSTextView)
        let before = currentPage(in: controller.pdfView)

        window.sendEvent(keyEvent(124, window: window))

        XCTAssertEqual(currentPage(in: controller.pdfView), before)
        XCTAssertTrue(window.firstResponder is NSTextView)
    }

    func testPDFViewCanBecomeFirstResponderForArrowNavigation() throws {
        let controller = try makeWorkbench(pageCount: 2)
        let window = try XCTUnwrap(controller.pdfView.window)
        XCTAssertTrue(window.makeFirstResponder(controller.pdfView))
        XCTAssertTrue(window.firstResponder === controller.pdfView)

        controller.pdfView.keyDown(with: keyEvent(124, window: window))
        XCTAssertEqual(currentPage(in: controller.pdfView), 1)
    }

    func testOverviewDoesNotInvalidateCachedThumbnailsForSameDocument() async throws {
        let document = Self.fixture(pageCount: 40)
        let controller = PageOverviewController()
        controller.loadView()

        controller.setDocument(document, selectedPage: 0)
        let size = NSSize(width: 180, height: 254)
        let first = await withCheckedContinuation { continuation in
            controller.renderThumbnailForTesting(page: 0, size: size) { image, onMain in
                continuation.resume(returning: (image, onMain))
            }
        }
        XCTAssertNotNil(first.0)
        XCTAssertFalse(first.1)

        controller.setDocument(document, selectedPage: 7)
        XCTAssertEqual(controller.selectedPages, IndexSet(integer: 7))
        let second = await withCheckedContinuation { continuation in
            controller.renderThumbnailForTesting(page: 0, size: size) { image, onMain in
                continuation.resume(returning: (image, onMain))
            }
        }
        XCTAssertNotNil(second.0)
        XCTAssertTrue(second.1, "same-document selection refresh must reuse the cached thumbnail")
    }

    func testOverviewDecodesSnapshotOnceAcrossMultiplePageThumbnails() async throws {
        let controller = PageOverviewController()
        controller.loadView()
        controller.setDocument(Self.fixture(pageCount: 40), selectedPage: 0)
        let size = NSSize(width: 180, height: 254)

        for page in [0, 1, 2] {
            let image = await withCheckedContinuation { continuation in
                controller.renderThumbnailForTesting(page: page, size: size) { image, _ in
                    continuation.resume(returning: image)
                }
            }
            XCTAssertNotNil(image)
        }

        XCTAssertEqual(controller.thumbnailDocumentDecodeCount, 1,
                       "the serial renderer should retain one generation-local PDFDocument")
    }

    func testOverviewUsesCallerSnapshotWithoutFallbackMainThreadSerialization() throws {
        let document = Self.fixture(pageCount: 40)
        let snapshot = try XCTUnwrap(document.dataRepresentation())
        let controller = PageOverviewController()
        controller.loadView()

        controller.setDocument(document, selectedPage: 0, snapshot: snapshot)

        XCTAssertEqual(controller.fallbackSnapshotSerializationCount, 0)
    }

    func testSameDocumentIdentityWithChangedSnapshotInvalidatesThumbnailGeneration() async throws {
        let document = Self.fixture(pageCount: 2)
        let originalSnapshot = try XCTUnwrap(document.dataRepresentation())
        let controller = PageOverviewController()
        controller.loadView()
        controller.setDocument(document, selectedPage: 0, snapshot: originalSnapshot)
        let size = NSSize(width: 180, height: 254)
        _ = await withCheckedContinuation { continuation in
            controller.renderThumbnailForTesting(page: 0, size: size) { image, _ in continuation.resume(returning: image) }
        }

        document.page(at: 0)?.rotation = 90
        let changedSnapshot = try XCTUnwrap(document.dataRepresentation())
        XCTAssertNotEqual(originalSnapshot, changedSnapshot)
        controller.setDocument(document, selectedPage: 0, snapshot: changedSnapshot)
        let refreshed = await withCheckedContinuation { continuation in
            controller.renderThumbnailForTesting(page: 0, size: size) { image, onMain in
                continuation.resume(returning: (image, onMain))
            }
        }

        XCTAssertNotNil(refreshed.0)
        XCTAssertFalse(refreshed.1, "changed bytes must invalidate the old synchronous cache hit")
    }

    func testSettingNilDocumentClearsRetainedWorkerDocument() async throws {
        let controller = PageOverviewController()
        controller.loadView()
        controller.setDocument(Self.fixture(pageCount: 2), selectedPage: 0)
        _ = await withCheckedContinuation { continuation in
            controller.renderThumbnailForTesting(page: 0, size: NSSize(width: 180, height: 254)) { image, _ in
                continuation.resume(returning: image)
            }
        }
        XCTAssertTrue(controller.thumbnailHasRetainedDocumentForTesting)

        controller.setDocument(nil, selectedPage: 0)
        controller.waitForThumbnailRendererForTesting()

        XCTAssertFalse(controller.thumbnailHasRetainedDocumentForTesting)
    }

    private func makeWorkbench(pageCount: Int) throws -> WorkbenchController {
        let controller = WorkbenchController()
        let window = NSWindow(
            contentRect: CGRect(x: 0, y: 0, width: 1280, height: 820),
            styleMask: [.titled], backing: .buffered, defer: false
        )
        window.contentViewController = controller
        try controller.workspace.createBlank()
        for index in 1..<pageCount { try controller.workspace.insertBlank(at: index) }
        controller.refreshDocument()
        controller.view.layoutSubtreeIfNeeded()
        addTeardownBlock { window.contentViewController = nil }
        return controller
    }

    private func keyEvent(_ keyCode: UInt16, window: NSWindow?) -> NSEvent {
        let characters: String
        switch keyCode {
        case 123: characters = "\u{F702}"
        case 124: characters = "\u{F703}"
        case 125: characters = "\u{F701}"
        default: characters = "\u{F700}"
        }
        return NSEvent.keyEvent(
            with: .keyDown,
            location: .zero,
            modifierFlags: [.numericPad],
            timestamp: 0,
            windowNumber: window?.windowNumber ?? 0,
            context: nil,
            characters: characters,
            charactersIgnoringModifiers: characters,
            isARepeat: false,
            keyCode: keyCode
        )!
    }

    private func currentPage(in view: PDFView) -> Int {
        guard let document = view.document, let page = view.currentPage else { return NSNotFound }
        return document.index(for: page)
    }

    private static func fixture(pageCount: Int) -> PDFDocument {
        let data = NSMutableData()
        let consumer = CGDataConsumer(data: data)!
        var media = CGRect(x: 0, y: 0, width: 420, height: 594)
        let context = CGContext(consumer: consumer, mediaBox: &media, nil)!
        for page in 0..<pageCount {
            context.beginPDFPage(nil)
            let text = NSAttributedString(string: "Page \(page + 1)", attributes: [.font: NSFont.systemFont(ofSize: 18)])
            CTLineDraw(CTLineCreateWithAttributedString(text), context)
            context.endPDFPage()
        }
        context.closePDF()
        return PDFDocument(data: data as Data)!
    }
}
