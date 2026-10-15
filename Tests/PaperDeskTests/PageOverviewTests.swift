import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

final class PageOverviewTests: XCTestCase {
    func testThumbnailPixelBudgetsRejectNonFiniteAndHugeAspectRatios() {
        for size in [NSSize(width: CGFloat.infinity, height: 20), NSSize(width: CGFloat.nan, height: 20), NSSize(width: 1e100, height: 1e100), NSSize(width: 360, height: 1e20)] {
            let bounded = PageOverviewController.boundedPixelSize(size)
            XCTAssertTrue(bounded.width.isFinite && bounded.height.isFinite)
            XCTAssertLessThanOrEqual(max(bounded.width, bounded.height), 4096)
            XCTAssertLessThanOrEqual(bounded.width * bounded.height, 4_005_000)
        }
    }
    @MainActor
    func testLayoutAdaptsColumnsAndThumbnailScale() {
        let controller = PageOverviewController()
        controller.loadView()
        controller.view.frame.size = NSSize(width: 760, height: 600)
        controller.view.layoutSubtreeIfNeeded()
        controller.refreshLayout()

        XCTAssertGreaterThan(controller.collectionView.frame.width, 700)
        XCTAssertEqual(controller.columns, 4)
        XCTAssertGreaterThan(controller.itemSize.width, 150)
        XCTAssertLessThan(controller.itemSize.width, 190)

        controller.setThumbnailScale(2)
        XCTAssertEqual(controller.thumbnailScale, 2)
        XCTAssertEqual(controller.columns, 2)

        controller.setThumbnailScale(99)
        XCTAssertEqual(controller.thumbnailScale, 2)
        controller.setThumbnailScale(0.1)
        XCTAssertEqual(controller.thumbnailScale, 0.5)
    }

    @MainActor
    func testSelectionSupportsToggleAndShiftRange() {
        let controller = PageOverviewController()
        controller.loadView()
        controller.setDocument(Self.fixture(pageCount: 8), selectedPage: 2)
        XCTAssertEqual(controller.selectedPages, IndexSet(integer: 2))

        controller.selectPage(5, extending: false, range: true)
        XCTAssertEqual(controller.selectedPages, IndexSet(integersIn: 2...5))

        controller.selectPage(7, extending: true, range: false)
        XCTAssertEqual(controller.selectedPages, IndexSet([2, 3, 4, 5, 7]))
        controller.selectPage(3, extending: true, range: false)
        XCTAssertEqual(controller.selectedPages, IndexSet([2, 4, 5, 7]))

        controller.restoreSelection(IndexSet([0, 4, 99]))
        XCTAssertEqual(controller.selectedPages, IndexSet([0, 4]))
    }

    @MainActor
    func testThumbnailRenderingUsesBoundedBackgroundSnapshotRenderer() async throws {
        let controller = PageOverviewController()
        controller.loadView()
        let document = Self.fixture(pageCount: 2)
        document.page(at: 1)?.rotation = 90
        controller.setDocument(document, selectedPage: 0)

        XCTAssertEqual(controller.thumbnailRendererMaximumConcurrency, 1)
        XCTAssertEqual(controller.thumbnailCacheCostLimit, 48 * 1_024 * 1_024)
        let requestSize = controller.thumbnailRequestSize(page: 1, itemWidth: 180)
        XCTAssertGreaterThan(requestSize.width, requestSize.height)

        let result = await withCheckedContinuation { continuation in
            controller.renderThumbnailForTesting(page: 1, size: requestSize) { image, renderedOnMainThread in
                continuation.resume(returning: (image, renderedOnMainThread))
            }
        }
        XCTAssertNotNil(result.0)
        XCTAssertFalse(result.1)
    }

    @MainActor
    func testSelectionIsClampedWhenPresentationChanges() {
        let controller = PageOverviewController()
        controller.loadView()
        var reported: IndexSet?
        controller.onSelectionChange = { reported = $0 }
        controller.setDocument(Self.fixture(pageCount: 3), selectedPage: 20)

        XCTAssertEqual(controller.selectedPages, IndexSet(integer: 2))
        XCTAssertEqual(reported, IndexSet(integer: 2))
        XCTAssertEqual(controller.numberOfSections(in: controller.collectionView), 1)
        XCTAssertEqual(controller.collectionView(controller.collectionView, numberOfItemsInSection: 0), 3)

        controller.setDocument(nil, selectedPage: 0)
        XCTAssertTrue(controller.selectedPages.isEmpty)
    }

    @MainActor
    func testOpenSelectedPageUsesZeroBasedPresentationPage() {
        let controller = PageOverviewController()
        controller.loadView()
        controller.setDocument(Self.fixture(pageCount: 4), selectedPage: 1)
        var opened: Int?
        controller.onOpenPage = { opened = $0 }

        controller.openPage(at: 3)
        XCTAssertEqual(opened, 3)
        controller.openPage(at: 9)
        XCTAssertEqual(opened, 3)
    }

    @MainActor
    func testItemSizePreservesPortraitPageSpaceAndNeverEagerlyCreatesItems() {
        let controller = PageOverviewController()
        controller.loadView()
        controller.view.frame.size = NSSize(width: 1000, height: 700)
        let document = Self.fixture(pageCount: 400)
        controller.setDocument(document, selectedPage: 0)
        controller.refreshLayout()

        XCTAssertGreaterThan(controller.itemSize.height, controller.itemSize.width)
        XCTAssertEqual(controller.collectionView.visibleItems().count, 0)
        XCTAssertEqual(controller.collectionView(controller.collectionView, numberOfItemsInSection: 0), 400)
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
