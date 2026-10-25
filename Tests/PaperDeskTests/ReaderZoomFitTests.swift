import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor
final class ReaderZoomFitTests: XCTestCase {
    func testFittedTwoPageSpreadIsFullyVisibleAfterSwitchingFromLargeZoom() async throws {
        let (reader, view) = try fixture(viewport: CGSize(width: 1000, height: 700),
            sizes: Array(repeating: CGSize(width: 600, height: 840), count: 4))
        try reader.setZoom(.percent(172))
        reader.setLayout(.twoPages, continuous: true)
        try reader.setZoom(.fitPage)
        await settle(view)
        let pages = try XCTUnwrap(view.document)
        let left = try XCTUnwrap(pages.page(at: 0)), right = try XCTUnwrap(pages.page(at: 1))
        let spread = view.convert(left.bounds(for: view.displayBox), from: left)
            .union(view.convert(right.bounds(for: view.displayBox), from: right))
        let clip = try XCTUnwrap(view.documentView?.enclosingScrollView?.contentView)
        let visible = view.convert(clip.bounds, from: clip)
        XCTAssertTrue(visible.insetBy(dx: -1, dy: -1).contains(spread), "spread \(spread), viewport \(visible)")
    }

    func testFitPageInContinuousModeFitsBothDimensionsWithConsistentBorder() async throws {
        let (reader, view) = try fixture(viewport: CGSize(width: 1000, height: 500))
        reader.setLayout(.single, continuous: true)
        try reader.setZoom(.fitPage)
        await settle(view)
        XCTAssertFalse(view.autoScales, "Continuous PDFKit autoScales is fit-width, not fit-page")
        try assertPageFits(view)
        let page = try XCTUnwrap(view.currentPage)
        XCTAssertEqual(view.convert(page.bounds(for: view.displayBox), from: page).height,
                       viewport(view).height - 32, accuracy: 0.5)
        page.rotation = 90
        view.layoutDocumentView()
        await settle(view)
        try assertPageFits(view)
        XCTAssertEqual(view.convert(page.bounds(for: view.displayBox), from: page).height,
                       viewport(view).height - 32, accuracy: 0.5)
    }

    func testFitWidthTracksActualWindowResizeAndSidebarViewportChanges() async throws {
        let (reader, view) = try fixture(viewport: CGSize(width: 900, height: 700))
        let window = NSWindow(contentRect: view.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let host = NSView(frame: view.frame)
        host.addSubview(view)
        view.autoresizingMask = [.width, .height]
        window.contentView = host
        defer { window.close() }
        try reader.setZoom(.fitWidth)
        await settle(view)
        let original = view.scaleFactor
        window.setContentSize(CGSize(width: 600, height: 700))
        await settle(view)
        XCTAssertLessThan(view.scaleFactor, original - 0.2)
        try assertWidthFits(view)
        let resized = view.scaleFactor
        view.frame = NSRect(x: 180, y: 0, width: 420, height: 700)
        await settle(view)
        XCTAssertLessThan(view.scaleFactor, resized - 0.1)
        try assertWidthFits(view)
    }

    func testFitWidthRecalculatesForPageNavigationRotationAndLayout() async throws {
        let (reader, view) = try fixture(viewport: CGSize(width: 900, height: 700),
                                       sizes: [CGSize(width: 600, height: 840), CGSize(width: 1200, height: 600)])
        reader.setLayout(.single, continuous: false)
        try reader.setZoom(.fitWidth)
        let original = view.scaleFactor
        try reader.goToPage(2)
        await settle(view)
        XCTAssertLessThan(view.scaleFactor, original * 0.6)
        try assertWidthFits(view)
        let page = try XCTUnwrap(view.currentPage)
        page.rotation = 90
        view.layoutDocumentView()
        await settle(view)
        XCTAssertGreaterThan(view.scaleFactor, original * 0.9)
        try assertWidthFits(view)
        reader.setLayout(.twoPages, continuous: false)
        await settle(view)
        XCTAssertEqual(view.displayMode, .twoUp)
        XCTAssertLessThan(view.scaleFactor, original * 0.8)
    }

    func testPercentCancelsFitAndRemainsFixedAcrossGeometryChanges() async throws {
        let (reader, view) = try fixture(viewport: CGSize(width: 900, height: 700))
        try reader.setZoom(.fitWidth)
        try reader.setZoom(.percent(137.5))
        view.frame.size = CGSize(width: 430, height: 350)
        view.bounds.size = CGSize(width: 410, height: 330)
        view.currentPage?.rotation = 90
        reader.setLayout(.twoPagesWithCover, continuous: true)
        await settle(view)
        XCTAssertEqual(view.scaleFactor, 1.375, accuracy: 1e-10)
        XCTAssertFalse(view.autoScales)
        XCTAssertThrowsError(try reader.setZoom(.percent(.nan)))
        XCTAssertEqual(view.scaleFactor, 1.375, accuracy: 1e-10)
    }

    func testFitPagePreservesNativeCoverAndSpreadLayoutWithoutHeightOverflow() async throws {
        let (reader, view) = try fixture(viewport: CGSize(width: 900, height: 450),
                                       sizes: Array(repeating: CGSize(width: 600, height: 840), count: 3))
        reader.setLayout(.twoPagesWithCover, continuous: true)
        try reader.setZoom(.fitPage)
        await settle(view)
        XCTAssertTrue(view.displaysAsBook)
        XCTAssertEqual(reader.currentLayout, .twoPagesWithCover)
        XCTAssertEqual(view.document?.index(for: try XCTUnwrap(view.currentPage)), 0)
        try assertPageFits(view)
        try reader.goToPage(2)
        await settle(view)
        XCTAssertTrue(view.displaysAsBook)
        try assertPageFits(view)
    }

    func testFitPageMakesTheCurrentPartiallyScrolledPageFullyVisible() async throws {
        let (reader, view) = try fixture(viewport: CGSize(width: 900, height: 450),
                                       sizes: Array(repeating: CGSize(width: 600, height: 840), count: 3))
        try reader.setZoom(.percent(140))
        let page = try XCTUnwrap(view.document?.page(at: 1))
        view.go(to: PDFDestination(page: page, at: CGPoint(x: 200, y: 400)))
        try reader.setZoom(.fitPage)
        await settle(view)
        try assertPageFits(view)
    }

    func testFitPageTracksHeightChangesWithoutLosingItsMode() async throws {
        let (reader, view) = try fixture(viewport: CGSize(width: 900, height: 650))
        try reader.setZoom(.fitPage)
        let original = view.scaleFactor
        view.frame.size = CGSize(width: 900, height: 350)
        await settle(view)
        XCTAssertEqual(reader.currentZoom, .fitPage)
        XCTAssertLessThan(view.scaleFactor, original * 0.6)
        try assertPageFits(view)
    }

    func testNativePageChangedNotificationRecalculatesFitWithoutReaderNavigation() async throws {
        let (reader, view) = try fixture(viewport: CGSize(width: 900, height: 650),
                                       sizes: [CGSize(width: 600, height: 840), CGSize(width: 1200, height: 600)])
        reader.setLayout(.single, continuous: false)
        try reader.setZoom(.fitWidth)
        let original = view.scaleFactor
        view.go(to: try XCTUnwrap(view.document?.page(at: 1)))
        await settle(view)
        XCTAssertLessThan(view.scaleFactor, original * 0.6)
        try assertWidthFits(view)
    }

    func testNativeDisplayBoxChangedNotificationRecalculatesUsingTheNewBox() async throws {
        let (reader, view) = try fixture(viewport: CGSize(width: 900, height: 650))
        let page = try XCTUnwrap(view.currentPage)
        page.setBounds(CGRect(x: 0, y: 0, width: 300, height: 420), for: .cropBox)
        view.layoutDocumentView()
        try reader.setZoom(.fitWidth)
        let croppedScale = view.scaleFactor
        view.displayBox = .mediaBox
        await settle(view)
        XCTAssertLessThan(view.scaleFactor, croppedScale * 0.6)
        try assertWidthFits(view)
    }

    func testReplacingPresentationReappliesFitAfterHostRestoresOldScale() async throws {
        let (reader, view) = try fixture(viewport: CGSize(width: 900, height: 650))
        let (_, replacement) = try fixture(viewport: CGSize(width: 900, height: 650),
                                           sizes: [CGSize(width: 1200, height: 840)])
        try reader.setZoom(.fitWidth)
        let oldScale = view.scaleFactor
        reader.setDocument(replacement.document, allowsCopying: true)
        // Workbench refreshDocument currently restores its captured scale after
        // replacing the presentation. The queued fit must win for a fit mode.
        view.scaleFactor = oldScale
        await settle(view)
        XCTAssertEqual(reader.currentZoom, .fitWidth)
        XCTAssertLessThan(view.scaleFactor, oldScale * 0.6)
        try assertWidthFits(view)
    }

    func testModeStateAndInvalidZoomRequestsPreserveTheLastValidChoice() async throws {
        let (reader, view) = try fixture(viewport: CGSize(width: 900, height: 650))
        XCTAssertNil(reader.currentZoom)
        try reader.setZoom(.fitPage)
        XCTAssertEqual(reader.currentZoom, .fitPage)
        let scale = view.scaleFactor
        XCTAssertThrowsError(try reader.setZoom(.percent(9)))
        XCTAssertEqual(reader.currentZoom, .fitPage)
        XCTAssertEqual(view.scaleFactor, scale)
        try reader.setZoom(.percent(175))
        XCTAssertEqual(reader.currentZoom, .percent(175))
        view.bounds.size = CGSize(width: 600, height: 420)
        await settle(view)
        XCTAssertEqual(view.scaleFactor, 1.75)
    }

    func testFitObserversDoNotRecursivelyScale() async throws {
        let (reader, view) = try fixture(viewport: CGSize(width: 900, height: 650))
        try reader.setZoom(.fitWidth)
        var scaleChanges = 0
        let token = NotificationCenter.default.addObserver(forName: .PDFViewScaleChanged,
            object: view, queue: .main) { _ in MainActor.assumeIsolated { scaleChanges += 1 } }
        defer { NotificationCenter.default.removeObserver(token) }
        view.frame.size = CGSize(width: 600, height: 500)
        await settle(view)
        let afterResize = scaleChanges
        await settle(view)
        XCTAssertLessThanOrEqual(afterResize, 3)
        XCTAssertEqual(scaleChanges, afterResize)
    }

    func testFitObserverAndPendingUpdateLifetimeIsWeak() async throws {
        let (_, view) = try fixture(viewport: CGSize(width: 900, height: 650))
        var reader: ReaderSession? = ReaderSession(pdfView: view)
        weak var released: ReaderSession?
        released = reader
        try reader?.setZoom(.fitPage)
        view.frame.size = CGSize(width: 600, height: 500)
        reader = nil
        XCTAssertNil(released)
        await settle(view)
        XCTAssertNil(released)
    }

    private func fixture(viewport: CGSize, sizes: [CGSize] = [CGSize(width: 600, height: 840)]) throws -> (ReaderSession, PDFView) {
        let bytes = NSMutableData()
        var defaultBox = CGRect(origin: .zero, size: sizes[0])
        let consumer = try XCTUnwrap(CGDataConsumer(data: bytes))
        let context = try XCTUnwrap(CGContext(consumer: consumer, mediaBox: &defaultBox, nil))
        for size in sizes {
            var box = CGRect(origin: .zero, size: size)
            let boxData = Data(bytes: &box, count: MemoryLayout<CGRect>.size)
            context.beginPDFPage([kCGPDFContextMediaBox as String: boxData] as CFDictionary)
            context.setFillColor(NSColor.white.cgColor)
            context.fill(box)
            context.setStrokeColor(NSColor.black.cgColor)
            context.stroke(box.insetBy(dx: 1, dy: 1))
            context.endPDFPage()
        }
        context.closePDF()
        let view = PDFView(frame: CGRect(origin: .zero, size: viewport))
        let reader = ReaderSession(pdfView: view)
        reader.setDocument(try XCTUnwrap(PDFDocument(data: bytes as Data)), allowsCopying: true)
        view.layoutSubtreeIfNeeded()
        view.layoutDocumentView()
        return (reader, view)
    }

    private func settle(_ view: PDFView) async {
        view.layoutSubtreeIfNeeded()
        view.layoutDocumentView()
        for _ in 0..<12 { await Task.yield() }
    }

    private func viewport(_ view: PDFView) -> CGSize {
        guard let clip = view.documentView?.enclosingScrollView?.contentView else { return view.bounds.size }
        return view.convert(clip.bounds, from: clip).intersection(view.bounds).size
    }

    private func assertWidthFits(_ view: PDFView, file: StaticString = #filePath, line: UInt = #line) throws {
        let page = try XCTUnwrap(view.currentPage, file: file, line: line)
        let pageRect = view.convert(page.bounds(for: view.displayBox), from: page)
        XCTAssertLessThanOrEqual(pageRect.width, viewport(view).width - 24, file: file, line: line)
        XCTAssertGreaterThan(pageRect.width, viewport(view).width - 70, file: file, line: line)
    }

    private func assertPageFits(_ view: PDFView, file: StaticString = #filePath, line: UInt = #line) throws {
        let page = try XCTUnwrap(view.currentPage, file: file, line: line)
        let rect = view.convert(page.bounds(for: view.displayBox), from: page)
        XCTAssertLessThanOrEqual(rect.width, viewport(view).width - 24, file: file, line: line)
        XCTAssertLessThanOrEqual(rect.height, viewport(view).height - 24, file: file, line: line)
        let visible: CGRect
        if let clip = view.documentView?.enclosingScrollView?.contentView {
            visible = view.convert(clip.bounds, from: clip)
        } else { visible = view.bounds }
        XCTAssertTrue(visible.insetBy(dx: -0.5, dy: -0.5).contains(rect),
                      "Whole current page must be visible: page \(rect), viewport \(visible)", file: file, line: line)
    }
}
