import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

final class PageOverviewBlackoutTests: XCTestCase {
    @MainActor
    func testUnrenderedGridItemHasVisiblePaperInsteadOfBlackCanvas() throws {
        let controller = PageOverviewController()
        controller.loadView()
        let document = Self.fixture(pageCount: 270)
        controller.setDocument(document, selectedPage: 2, snapshot: try XCTUnwrap(document.dataRepresentation()))
        let item = controller.collectionView(controller.collectionView,
                                             itemForRepresentedObjectAt: IndexPath(item: 2, section: 0))
        let imageView = try XCTUnwrap(item.view.subviews.compactMap { $0 as? NSImageView }.first)
        XCTAssertNotNil(imageView.layer?.backgroundColor,
                        "An asynchronous thumbnail must reserve a visible paper surface, not disappear into black canvas.")
        if let color = imageView.layer?.backgroundColor {
            let gray = try XCTUnwrap(NSColor(cgColor: color)?.usingColorSpace(.deviceRGB))
            XCTAssertGreaterThan(gray.redComponent, 0.85)
        }
        controller.waitForThumbnailRendererForTesting()
    }

    @MainActor
    func testMutationRefreshPublishesRealThumbnailsForLargeGrid() async throws {
        let controller = PageOverviewController()
        controller.loadView()
        let original = Self.fixture(pageCount: 270)
        controller.setDocument(original, selectedPage: 2, snapshot: try XCTUnwrap(original.dataRepresentation()))
        let first = await thumbnail(controller, page: 2)
        XCTAssertNotNil(first)
        let modified = try XCTUnwrap(PDFDocument(data: try XCTUnwrap(original.dataRepresentation())))
        modified.insert(try XCTUnwrap(original.page(at: 2)?.copy() as? PDFPage), at: 3)
        controller.setDocument(modified, selectedPage: 3, snapshot: try XCTUnwrap(modified.dataRepresentation()))
        for page in [0, 3, 8, 18, 26, 35, 270] {
            let rendered = await thumbnail(controller, page: page)
            let image = try XCTUnwrap(rendered)
            XCTAssertNotNil(image.cgImage(forProposedRect: nil, context: nil, hints: nil))
        }
        XCTAssertEqual(controller.thumbnailDocumentDecodeCount, 2)
        XCTAssertEqual(controller.collectionView(controller.collectionView, numberOfItemsInSection: 0), 271)
        XCTAssertEqual(controller.selectedPages, [3])
    }

    @MainActor
    private func thumbnail(_ controller: PageOverviewController, page: Int) async -> NSImage? {
        await withCheckedContinuation { continuation in
            controller.renderThumbnailForTesting(page: page, size: NSSize(width: 180, height: 255)) { image, _ in
                continuation.resume(returning: image)
            }
        }
    }

    private static func fixture(pageCount: Int) -> PDFDocument {
        let data = NSMutableData()
        var media = CGRect(x: 0, y: 0, width: 420, height: 594)
        let context = CGContext(consumer: CGDataConsumer(data: data)!, mediaBox: &media, nil)!
        for page in 0..<pageCount {
            context.beginPDFPage(nil)
            context.setFillColor(CGColor(gray: 1, alpha: 1)); context.fill(media)
            context.setFillColor(CGColor(red: CGFloat(page % 5) / 6, green: 0.2, blue: 0.5, alpha: 1))
            context.fill(CGRect(x: 24, y: 48, width: 300, height: 200))
            context.endPDFPage()
        }
        context.closePDF()
        return PDFDocument(data: data as Data)!
    }
}
