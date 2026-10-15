import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor final class UndoPresentationTests: XCTestCase {
    func testInkUndoRedoKeepsReaderPageZoomAndScrollPosition() throws {
        let controller = WorkbenchController(); _ = controller.view
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1100, height: 820), styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.contentViewController = controller
        try controller.workspace.createBlank(); controller.refreshDocument(); controller.enterMode(.annotation)
        controller.toggleInk(NSMenuItem())
        controller.view.layoutSubtreeIfNeeded()
        controller.pdfView.scaleFactor = 2.75
        let displayed = try XCTUnwrap(controller.pdfView.document)
        let page = try XCTUnwrap(displayed.page(at: 0))
        controller.pdfView.beginGesture(on: page, at: CGPoint(x: 80, y: 110))
        controller.pdfView.updateGesture(to: CGPoint(x: 180, y: 160)); controller.pdfView.finishGesture()
        let scroll = try XCTUnwrap(descendants(controller.pdfView).compactMap { $0 as? NSScrollView }.first)
        scroll.contentView.scroll(to: CGPoint(x: 90, y: 220)); scroll.reflectScrolledClipView(scroll.contentView)
        let origin = scroll.contentView.bounds.origin
        for count in [0, 1, 0, 1] {
            if count == 0 { controller.undo() } else { controller.redo() }
            XCTAssertTrue(controller.pdfView.document === displayed, "Undo must not replace PDFView.document")
            XCTAssertTrue(controller.pdfView.document?.page(at: 0) === page, "Page tiles must retain their page")
            XCTAssertEqual(page.annotations.filter { $0.type == "Ink" }.count, count)
            XCTAssertEqual(controller.workspace.document?.page(at: 0)?.annotations.filter { $0.type == "Ink" }.count, count)
            XCTAssertEqual(controller.pdfView.scaleFactor, 2.75, accuracy: 0.001)
            XCTAssertEqual(scroll.contentView.bounds.origin.x, origin.x, accuracy: 0.001)
            XCTAssertEqual(scroll.contentView.bounds.origin.y, origin.y, accuracy: 0.001)
        }
        withExtendedLifetime(window) {}
    }

    func testEmptyUndoAndRedoDoNotRebindReader() throws {
        let controller = WorkbenchController(); _ = controller.view
        try controller.workspace.createBlank(); controller.refreshDocument()
        let displayed = controller.pdfView.document
        let revision = controller.workspace.revision
        controller.undo(); controller.redo()
        XCTAssertTrue(controller.pdfView.document === displayed)
        XCTAssertEqual(controller.workspace.revision, revision)
    }

    func testPageStructureUndoRedoStillRefreshesPageCount() throws {
        let controller = WorkbenchController(); _ = controller.view
        try controller.workspace.createBlank(); controller.refreshDocument()
        try controller.workspace.insertBlank(at: 1); controller.refreshDocument()
        controller.undo()
        XCTAssertEqual(controller.pdfView.document?.pageCount, 1)
        controller.redo()
        XCTAssertEqual(controller.pdfView.document?.pageCount, 2)
    }

    func testImportedAppearanceSurvivesRepeatedUndoRedoAndReaderRelease() async throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("UndoAppearance-\(UUID().uuidString).pdf")
        try nativeAppearanceFixture().write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }
        for _ in 0..<8 {
            try autoreleasepool {
                let controller = WorkbenchController(); _ = controller.view
                try controller.workspace.open(url); controller.refreshDocument()
                let page = try XCTUnwrap(controller.pdfView.document?.page(at: 0))
                let original = try pixels(page)
                XCTAssertGreaterThan(try bluePixelCount(page), 0, "Fixture must render its blue /AP, not its red /C property")
                try controller.workspace.addInk(points: [CGPoint(x: 240, y: 300), CGPoint(x: 280, y: 350)], page: 0, color: .red, width: 3)
                controller.refreshDocument()
                let displayed = try XCTUnwrap(controller.pdfView.document)
                let target = try XCTUnwrap(displayed.page(at: 0))
                for _ in 0..<4 {
                    controller.undo()
                    XCTAssertTrue(controller.pdfView.document === displayed)
                    XCTAssertEqual(target.annotations.count, 1)
                    XCTAssertEqual(try pixels(target), original, "Undo must retain the imported custom blue appearance, not regenerate from its red /C entry")
                    XCTAssertEqual(controller.workspace.document?.page(at: 0)?.annotations.count, 1)
                    controller.redo()
                    XCTAssertEqual(target.annotations.count, 2)
                    XCTAssertEqual(controller.workspace.document?.page(at: 0)?.annotations.count, 2)
                    XCTAssertNotNil(displayed.dataRepresentation())
                }
            }
            // Let PDFKit's asynchronous native appearance processing finish after
            // both controller and removed annotation owners have been released.
            try await Task.sleep(nanoseconds: 10_000_000)
        }
    }

    private func pixels(_ page: PDFPage) throws -> Data {
        let tiff = try XCTUnwrap(page.thumbnail(of: NSSize(width: 306, height: 396), for: .cropBox).tiffRepresentation)
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: tiff))
        let bytes = try XCTUnwrap(bitmap.bitmapData)
        return Data(bytes: bytes, count: bitmap.bytesPerRow * bitmap.pixelsHigh)
    }

    private func bluePixelCount(_ page: PDFPage) throws -> Int {
        let tiff = try XCTUnwrap(page.thumbnail(of: NSSize(width: 306, height: 396), for: .cropBox).tiffRepresentation)
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: tiff))
        var count = 0
        for y in stride(from: 0, to: bitmap.pixelsHigh, by: 5) {
            for x in stride(from: 0, to: bitmap.pixelsWide, by: 5) {
                if let c = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB),
                   c.blueComponent > c.redComponent + 0.3, c.blueComponent > c.greenComponent + 0.3 { count += 1 }
            }
        }
        return count
    }

    private func nativeAppearanceFixture() -> Data {
        let stream = "0 0 1 rg 0 0 80 60 re f\n"
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Annots [4 0 R] >>",
            "<< /Type /Annot /Subtype /Square /Rect [50 50 130 110] /C [1 0 0] /Border [0 0 2] /AP << /N 5 0 R >> >>",
            "<< /Type /XObject /Subtype /Form /BBox [0 0 80 60] /Resources << >> /Length \(stream.utf8.count) >>\nstream\n\(stream)endstream"
        ]
        var pdf = "%PDF-1.4\n", offsets = [0]
        for (index, object) in objects.enumerated() {
            offsets.append(pdf.utf8.count); pdf += "\(index + 1) 0 obj\n\(object)\nendobj\n"
        }
        let xref = pdf.utf8.count
        pdf += "xref\n0 \(objects.count + 1)\n0000000000 65535 f \n"
        for offset in offsets.dropFirst() { pdf += String(format: "%010d 00000 n \n", offset) }
        pdf += "trailer\n<< /Size \(objects.count + 1) /Root 1 0 R >>\nstartxref\n\(xref)\n%%EOF\n"
        return Data(pdf.utf8)
    }

    private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
}
