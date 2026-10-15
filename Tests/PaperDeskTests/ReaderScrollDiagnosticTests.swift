import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

/// Native PDFKit diagnostic: intentionally separate from the fix owner's tests.
@MainActor
final class ReaderScrollDiagnosticTests: XCTestCase {
    func testFixedPercentControlPureClipOriginScrollDoesNotRequestRelayout() async throws {
        let (reader, view, window) = try fixture()
        defer { window.orderOut(nil); _ = reader }
        await settle()
        try reader.setZoom(.percent(Double(view.scaleFactor * 100)))
        await settle()
        let clip = try XCTUnwrap(view.documentView?.enclosingScrollView?.contentView)
        let requested = NSPoint(x: clip.bounds.origin.x, y: clip.bounds.origin.y + 30)
        view.resetDiagnosticCounts()

        clip.scroll(to: requested)
        clip.enclosingScrollView?.reflectScrolledClipView(clip)
        await settle()

        print("SCROLL_DIAGNOSTIC fixedPercent requested=\(requested) after=\(clip.bounds) layout=\(view.layoutRequests) reveal=\(view.revealRequests) scaleWrites=\(view.scaleWrites)")
        XCTAssertEqual(view.layoutRequests, 0)
        XCTAssertEqual(view.revealRequests, 0)
        XCTAssertEqual(view.scaleWrites, 0)
        XCTAssertEqual(clip.bounds.origin.y, requested.y, accuracy: 0.5)
    }

    func testFitPagePureClipOriginScrollDoesNotRequestDocumentRelayoutOrPageReveal() async throws {
        let (reader, view, window) = try fixture()
        defer { window.orderOut(nil); _ = reader }
        await settle()
        let clip = try XCTUnwrap(view.documentView?.enclosingScrollView?.contentView)
        let before = clip.bounds
        let requested = NSPoint(x: before.origin.x, y: before.origin.y + 30)
        view.resetDiagnosticCounts()

        clip.scroll(to: requested)
        clip.enclosingScrollView?.reflectScrolledClipView(clip)
        await settle()

        print("SCROLL_DIAGNOSTIC small before=\(before) requested=\(requested) after=\(clip.bounds) layout=\(view.layoutRequests) reveal=\(view.revealRequests) scaleWrites=\(view.scaleWrites) current=\(view.currentPage.flatMap { view.document?.index(for: $0) } ?? -1)")
        XCTAssertEqual(view.layoutRequests, 0, "Changing only scroll origin is not a fit geometry change")
        XCTAssertEqual(view.revealRequests, 0, "A manual scroll must not call go(to:pageBounds,on:) to snap back")
        XCTAssertEqual(view.scaleWrites, 0)
        XCTAssertEqual(clip.bounds.origin.y, requested.y, accuracy: 0.5)
    }

    func testNativeContinuousPageChangeAtUnchangedScaleDoesNotApplyFitReveal() async throws {
        let (reader, view, window) = try fixture()
        defer { window.orderOut(nil); _ = reader }
        await settle()
        let clip = try XCTUnwrap(view.documentView?.enclosingScrollView?.contentView)
        let priorPage = view.currentPage.flatMap { view.document?.index(for: $0) } ?? -1
        let next = try XCTUnwrap(view.document?.page(at: 1))
        // Hidden PDFKit windows do not update currentPage from a direct clip
        // scroll. Use native destination navigation to emit the same page-change
        // notification and inspect only the observer's subsequent work.
        view.go(to: PDFDestination(page: next, at: CGPoint(x: 200, y: 400)))
        let requested = clip.bounds.origin
        view.resetDiagnosticCounts()
        await settle()

        let afterPage = view.currentPage.flatMap { view.document?.index(for: $0) } ?? -1
        print("SCROLL_DIAGNOSTIC pageChange requested=\(requested) after=\(clip.bounds) layout=\(view.layoutRequests) reveal=\(view.revealRequests) scaleWrites=\(view.scaleWrites) page=\(priorPage)->\(afterPage)")
        XCTAssertNotEqual(afterPage, priorPage, "Fixture must cross a native page boundary")
        XCTAssertEqual(view.revealRequests, 0, "Equal-sized page indices must not force-fit and reveal the new page during scrolling")
        XCTAssertEqual(view.scaleWrites, 0)
        XCTAssertEqual(clip.bounds.origin.y, requested.y, accuracy: 0.5)
    }

    func testFitWidthRepeatedBidirectionalScrollAndDuplicatePageNotificationsDoNotRelayout() async throws {
        let (reader, view, window) = try fixture(zoom: .fitWidth)
        defer { window.orderOut(nil); _ = reader }
        await settle()
        let clip = try XCTUnwrap(view.documentView?.enclosingScrollView?.contentView)
        let initial = clip.bounds.origin
        let initialScale = view.scaleFactor

        for offset: CGFloat in [30, 60, 15, 45, 0] {
            let requested = NSPoint(x: initial.x, y: initial.y + offset)
            view.resetDiagnosticCounts()
            clip.scroll(to: requested)
            clip.enclosingScrollView?.reflectScrolledClipView(clip)
            NotificationCenter.default.post(name: .PDFViewPageChanged, object: view)
            NotificationCenter.default.post(name: .PDFViewPageChanged, object: view)
            await settle()

            XCTAssertEqual(view.layoutRequests, 0, "FitWidth offset \(offset)")
            XCTAssertEqual(view.revealRequests, 0)
            XCTAssertEqual(view.scaleWrites, 0)
            XCTAssertEqual(view.scaleFactor, initialScale, accuracy: 1e-9)
            XCTAssertEqual(clip.bounds.origin.y, requested.y, accuracy: 0.5)
        }
        XCTAssertEqual(reader.currentZoom, .fitWidth)
    }

    func testFitWidthEqualSizedNativePageChangesKeepScaleAndScrollAnchor() async throws {
        let (reader, view, window) = try fixture(zoom: .fitWidth)
        defer { window.orderOut(nil); _ = reader }
        await settle()
        let initialScale = view.scaleFactor
        let clip = try XCTUnwrap(view.documentView?.enclosingScrollView?.contentView)

        for index in [1, 2, 1, 0] {
            let page = try XCTUnwrap(view.document?.page(at: index))
            view.go(to: PDFDestination(page: page, at: CGPoint(x: 200, y: 400)))
            let requested = clip.bounds.origin
            view.resetDiagnosticCounts()
            await settle()

            XCTAssertEqual(view.document?.index(for: try XCTUnwrap(view.currentPage)), index)
            XCTAssertEqual(view.layoutRequests, 0)
            XCTAssertEqual(view.revealRequests, 0)
            XCTAssertEqual(view.scaleWrites, 0)
            XCTAssertEqual(view.scaleFactor, initialScale, accuracy: 1e-9)
            XCTAssertEqual(clip.bounds.origin.y, requested.y, accuracy: 0.5)
        }
        XCTAssertEqual(reader.currentZoom, .fitWidth)
    }

    func testFixedPercentStaysFixedAcrossMixedPageSizesAndBidirectionalScroll() async throws {
        let sizes = [CGSize(width: 1200, height: 800), CGSize(width: 600, height: 1200), CGSize(width: 1600, height: 900)]
        let (reader, view, window) = try fixture(sizes: sizes, zoom: .percent(87.5))
        defer { window.orderOut(nil); _ = reader }
        await settle()
        let clip = try XCTUnwrap(view.documentView?.enclosingScrollView?.contentView)

        for index in [1, 2, 0] {
            let page = try XCTUnwrap(view.document?.page(at: index))
            view.go(to: PDFDestination(page: page, at: CGPoint(x: 200, y: 400)))
            let initial = clip.bounds.origin
            for offset: CGFloat in [20, 5] {
                let requested = NSPoint(x: initial.x, y: initial.y + offset)
                view.resetDiagnosticCounts()
                clip.scroll(to: requested)
                clip.enclosingScrollView?.reflectScrolledClipView(clip)
                NotificationCenter.default.post(name: .PDFViewPageChanged, object: view)
                await settle()

                XCTAssertEqual(view.layoutRequests, 0)
                XCTAssertEqual(view.revealRequests, 0)
                XCTAssertEqual(view.scaleWrites, 0)
                XCTAssertEqual(view.scaleFactor, 0.875, accuracy: 1e-9)
                XCTAssertEqual(clip.bounds.origin.y, requested.y, accuracy: 0.5)
            }
        }
        XCTAssertEqual(reader.currentZoom, .percent(87.5))
    }

    func testMixedSizePassivePageChangesStillRecalculateFitWithoutExplicitPageReveal() async throws {
        let sizes = [CGSize(width: 1200, height: 800), CGSize(width: 600, height: 1200), CGSize(width: 1600, height: 900)]
        for zoom in [ReaderZoom.fitPage, .fitWidth] {
            let (reader, view, window) = try fixture(sizes: sizes, zoom: zoom)
            defer { window.orderOut(nil); _ = reader }
            await settle()

            for index in [1, 2, 0] {
                let page = try XCTUnwrap(view.document?.page(at: index))
                let oldScale = view.scaleFactor
                view.go(to: PDFDestination(page: page, at: CGPoint(x: 200, y: sizes[index].height / 2)))
                view.resetDiagnosticCounts()
                await settle()

                let clip = try XCTUnwrap(view.documentView?.enclosingScrollView?.contentView)
                let measured = view.convert(clip.bounds, from: clip).intersection(view.bounds).size
                let backing = window.backingScaleFactor
                let viewport = CGSize(width: (measured.width * backing).rounded() / backing,
                                      height: (measured.height * backing).rounded() / backing)
                let widthFit = (viewport.width - 32) / sizes[index].width
                let expected = zoom == .fitWidth ? widthFit : min(widthFit, (viewport.height - 32) / sizes[index].height)
                XCTAssertEqual(view.document?.index(for: try XCTUnwrap(view.currentPage)), index)
                XCTAssertEqual(view.scaleFactor, expected, accuracy: 0.005, "\(zoom), page \(index)")
                XCTAssertGreaterThan(abs(view.scaleFactor - oldScale), 0.01, "Fixture must actually require a new scale")
                XCTAssertGreaterThan(view.scaleWrites, 0)
                XCTAssertLessThanOrEqual(view.scaleWrites, 2, "Scale changes must settle instead of recurring")
                XCTAssertEqual(view.revealRequests, 0, "Passive resize-to-page must not issue go(fullPage)")
                XCTAssertEqual(reader.currentZoom, zoom)
            }
        }
    }

    private func fixture(sizes: [CGSize] = Array(repeating: CGSize(width: 1200, height: 800), count: 5),
                         zoom: ReaderZoom = .fitPage) throws -> (ReaderSession, ScrollDiagnosticPDFView, NSWindow) {
        _ = NSApplication.shared
        let bytes = NSMutableData()
        var box = CGRect(origin: .zero, size: sizes[0])
        let consumer = try XCTUnwrap(CGDataConsumer(data: bytes))
        let context = try XCTUnwrap(CGContext(consumer: consumer, mediaBox: &box, nil))
        for size in sizes {
            box = CGRect(origin: .zero, size: size)
            let pageBox = Data(bytes: &box, count: MemoryLayout<CGRect>.size)
            context.beginPDFPage([kCGPDFContextMediaBox as String: pageBox] as CFDictionary)
            context.setFillColor(NSColor.white.cgColor)
            context.fill(box)
            context.endPDFPage()
        }
        context.closePDF()
        let view = ScrollDiagnosticPDFView(frame: NSRect(x: 0, y: 0, width: 1000, height: 680))
        let window = NSWindow(contentRect: view.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = view
        let reader = ReaderSession(pdfView: view)
        reader.setDocument(try XCTUnwrap(PDFDocument(data: bytes as Data)), allowsCopying: true)
        reader.setLayout(.single, continuous: true)
        try reader.setZoom(zoom)
        return (reader, view, window)
    }

    private func settle() async {
        for _ in 0..<4 { try? await Task.sleep(nanoseconds: 10_000_000) }
    }
}

@MainActor
private final class ScrollDiagnosticPDFView: PDFView {
    private(set) var layoutRequests = 0
    private(set) var revealRequests = 0
    private(set) var scaleWrites = 0

    override func layoutDocumentView() {
        layoutRequests += 1
        super.layoutDocumentView()
    }

    override func go(to rect: NSRect, on page: PDFPage) {
        revealRequests += 1
        super.go(to: rect, on: page)
    }

    override var scaleFactor: CGFloat {
        get { super.scaleFactor }
        set { scaleWrites += 1; super.scaleFactor = newValue }
    }

    func resetDiagnosticCounts() {
        layoutRequests = 0
        revealRequests = 0
        scaleWrites = 0
    }
}
