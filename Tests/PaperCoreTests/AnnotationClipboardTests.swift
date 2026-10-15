import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor final class AnnotationClipboardTests: XCTestCase {
    func testSquareRoundTripIsIndependentAndOneUndo() throws {
        let source = PDFWorkspace(); try source.createBlank()
        let a = PDFAnnotation(bounds: CGRect(x: 40, y: 50, width: 90, height: 60), forType: .square, withProperties: nil)
        a.color = .red; a.interiorColor = .blue; a.contents = "copied"
        source.document?.page(at: 0)?.addAnnotation(a)
        let bytes = try source.copyAnnotation(page: 0, index: 0)
        XCTAssertFalse(source.canUndo)
        let target = PDFWorkspace(); try target.createBlank()
        let index = try target.pasteAnnotation(bytes, page: 0)
        let pasted = try XCTUnwrap(target.document?.page(at: 0)?.annotations[index])
        XCTAssertEqual(pasted.type, "Square"); XCTAssertEqual(pasted.contents, "copied")
        XCTAssertEqual(pasted.bounds.size, a.bounds.size)
        XCTAssertEqual(pasted.color.usingColorSpace(.deviceRGB), a.color.usingColorSpace(.deviceRGB))
        XCTAssertEqual(pasted.interiorColor?.usingColorSpace(.deviceRGB), a.interiorColor?.usingColorSpace(.deviceRGB))
        pasted.contents = "independent"; XCTAssertEqual(a.contents, "copied")
        XCTAssertEqual(target.undo(), .annotations([0])); XCTAssertFalse(target.canUndo)
        XCTAssertTrue(target.document?.page(at: 0)?.annotations.isEmpty == true)
        target.redo(); XCTAssertEqual(target.document?.page(at: 0)?.annotations.count, 1)
    }

    func testRejectsUnsupportedAndActiveAnnotations() throws {
        for type: PDFAnnotationSubtype in [.link, .widget, .stamp, .square] {
            let source = PDFWorkspace(); try source.createBlank()
            let a = PDFAnnotation(bounds: CGRect(x: 40, y: 50, width: 90, height: 60), forType: type, withProperties: nil)
            if type == .square { a.action = PDFActionURL(url: URL(string: "https://example.com")!) }
            source.document?.page(at: 0)?.addAnnotation(a)
            XCTAssertThrowsError(try source.copyAnnotation(page: 0, index: 0))
            XCTAssertFalse(source.canUndo)
        }
    }

    func testMalformedAndOversizedCarrierPreserveHistory() throws {
        let target = PDFWorkspace(); try target.createBlank()
        try target.addText("redo", page: 0, bounds: CGRect(x: 10, y: 10, width: 80, height: 30))
        target.undo(); let revision = target.revision; let document = target.document
        for data in [Data("bad".utf8), Data(repeating: 0, count: 8 * 1_024 * 1_024 + 1)] {
            XCTAssertThrowsError(try target.pasteAnnotation(data, page: 0))
            XCTAssertTrue(target.document === document); XCTAssertEqual(target.revision, revision)
            XCTAssertTrue(target.canRedo); XCTAssertFalse(target.canUndo)
        }
    }

    func testInkAndMarkupGeometryAndOpacitySurviveCarrier() throws {
        for type: PDFAnnotationSubtype in [.ink, .highlight, .underline, .strikeOut, .line, .freeText, .circle, .text] {
            let source = PDFWorkspace(); try source.createBlank()
            let a = PDFAnnotation(bounds: CGRect(x: 40, y: 50, width: 90, height: 60), forType: type, withProperties: nil)
            a.color = NSColor.red.withAlphaComponent(0.45)
            if type == .ink { let p = NSBezierPath(); p.move(to: CGPoint(x: 3, y: 4)); p.line(to: CGPoint(x: 60, y: 30)); a.add(p) }
            if [.highlight, .underline, .strikeOut].contains(type) {
                a.quadrilateralPoints = [NSValue(point: CGPoint(x: 0, y: 50)), NSValue(point: CGPoint(x: 90, y: 50)), NSValue(point: .zero), NSValue(point: CGPoint(x: 90, y: 0))]
            }
            if type == .line { a.startPoint = CGPoint(x: 3, y: 4); a.endPoint = CGPoint(x: 60, y: 30); a.endLineStyle = .openArrow }
            if type == .freeText { a.font = .systemFont(ofSize: 17); a.contents = "text" }
            source.document?.page(at: 0)?.addAnnotation(a)
            let data = try source.copyAnnotation(page: 0, index: 0)
            let target = PDFWorkspace(); try target.createBlank()
            let index = try target.pasteAnnotation(data, page: 0)
            let p = try XCTUnwrap(target.document?.page(at: 0)?.annotations[index])
            XCTAssertEqual(p.type, a.type)
            if type == .ink { XCTAssertEqual(p.paths?.count, 1); XCTAssertEqual(p.paths?.first?.elementCount, 2) }
            if [.highlight, .underline, .strikeOut].contains(type) { XCTAssertEqual(p.quadrilateralPoints, a.quadrilateralPoints) }
            if type == .line { XCTAssertEqual(p.startPoint, a.startPoint); XCTAssertEqual(p.endPoint, a.endPoint); XCTAssertEqual(p.endLineStyle, a.endLineStyle) }
            if type != .text { XCTAssertEqual(AnnotationStyle(annotation: p).strokeColor.alphaComponent, 0.45, accuracy: 0.02, "\(type)") }
        }
    }

    func testEncryptedCopyAndNonCommentingPasteAreDenied() throws {
        let source = PDFWorkspace(); try source.createBlank()
        let a = PDFAnnotation(bounds: CGRect(x: 40, y: 50, width: 90, height: 60), forType: .square, withProperties: nil)
        source.document?.page(at: 0)?.addAnnotation(a)
        let bytes = try source.copyAnnotation(page: 0, index: 0)
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("AnnotationClipboard-protected-\(UUID().uuidString).pdf")
        defer { try? FileManager.default.removeItem(at: url) }
        XCTAssertTrue(source.document!.write(to: url, withOptions: [.ownerPasswordOption: "test-owner", .accessPermissionsOption: NSNumber(value: 0)]))
        let restricted = PDFWorkspace(); try restricted.open(url)
        XCTAssertTrue(restricted.isEncryptedSource); XCTAssertFalse(restricted.allowsCommenting)
        let original = restricted.document
        XCTAssertThrowsError(try restricted.copyAnnotation(page: 0, index: 0)) { XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied) }
        XCTAssertThrowsError(try restricted.pasteAnnotation(bytes, page: 0)) { XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied) }
        XCTAssertTrue(restricted.document === original); XCTAssertFalse(restricted.canUndo); XCTAssertFalse(restricted.isDirty)
    }

    func testTranslationClampsToCropWithoutChangingLineGeometry() throws {
        let source = PDFWorkspace(); try source.createBlank()
        let a = PDFAnnotation(bounds: CGRect(x: 400, y: 600, width: 90, height: 60), forType: .line, withProperties: nil)
        a.startPoint = CGPoint(x: 5, y: 10); a.endPoint = CGPoint(x: 80, y: 40)
        source.document?.page(at: 0)?.addAnnotation(a)
        let data = try source.copyAnnotation(page: 0, index: 0)
        let target = PDFWorkspace(); try target.createBlank()
        let crop = CGRect(x: 20, y: 30, width: 150, height: 120)
        target.document?.page(at: 0)?.setBounds(crop, for: .cropBox)
        let index = try target.pasteAnnotation(data, page: 0)
        let pasted = try XCTUnwrap(target.document?.page(at: 0)?.annotations[index])
        XCTAssertTrue(crop.contains(pasted.bounds)); XCTAssertEqual(pasted.bounds.size, a.bounds.size)
        XCTAssertEqual(pasted.startPoint, a.startPoint); XCTAssertEqual(pasted.endPoint, a.endPoint)
    }

    func testOversizedAnnotationAndWrongPagePreserveDestination() throws {
        let source = PDFWorkspace(); try source.createBlank()
        source.document?.page(at: 0)?.addAnnotation(PDFAnnotation(bounds: CGRect(x: 0, y: 0, width: 500, height: 600), forType: .square, withProperties: nil))
        let bytes = try source.copyAnnotation(page: 0, index: 0)
        let target = PDFWorkspace(); try target.createBlank()
        target.document?.page(at: 0)?.setBounds(CGRect(x: 0, y: 0, width: 100, height: 100), for: .cropBox)
        let original = target.document
        XCTAssertThrowsError(try target.pasteAnnotation(bytes, page: 0))
        XCTAssertThrowsError(try target.pasteAnnotation(bytes, page: -1))
        XCTAssertTrue(target.document === original); XCTAssertFalse(target.canUndo); XCTAssertEqual(target.revision, 0)
    }

    func testActiveAndMultipleObjectCarriersAreRejectedWithoutMutation() throws {
        let target = PDFWorkspace(); try target.createBlank()
        let original = target.document
        for kind in [0, 1, 2, 3] {
            let carrier = PDFDocument(), page = PDFPage()
            page.setBounds(CGRect(x: 0, y: 0, width: 612, height: 792), for: .mediaBox)
            let a = PDFAnnotation(bounds: CGRect(x: 40, y: 50, width: 90, height: 60), forType: kind == 3 ? .link : .square, withProperties: nil)
            if kind == 0 { a.action = PDFActionURL(url: URL(string: "https://example.com")!) }
            if kind == 1 { a.action = PDFActionGoTo(destination: PDFDestination(page: page, at: .zero)) }
            page.addAnnotation(a)
            if kind == 2 { page.addAnnotation(PDFAnnotation(bounds: a.bounds, forType: .circle, withProperties: nil)) }
            carrier.insert(page, at: 0)
            XCTAssertThrowsError(try target.pasteAnnotation(XCTUnwrap(carrier.dataRepresentation()), page: 0))
            XCTAssertTrue(target.document === original); XCTAssertFalse(target.canUndo); XCTAssertEqual(target.revision, 0)
        }
    }

    func testCopyExportsOnlySelectedAnnotationWithoutSourcePageContents() throws {
        let source = PDFWorkspace(); try source.createBlank()
        try source.addText("not selected", page: 0, bounds: CGRect(x: 40, y: 50, width: 100, height: 50))
        let a = PDFAnnotation(bounds: CGRect(x: 250, y: 50, width: 90, height: 60), forType: .square, withProperties: nil)
        a.contents = "selected"; source.document?.page(at: 0)?.addAnnotation(a)
        let carrier = try XCTUnwrap(PDFDocument(data: source.copyAnnotation(page: 0, index: 1)))
        XCTAssertEqual(carrier.pageCount, 1); XCTAssertEqual(carrier.page(at: 0)?.annotations.count, 1)
        XCTAssertEqual(carrier.page(at: 0)?.annotations.first?.contents, "selected")
        XCTAssertTrue(carrier.page(at: 0)?.string?.isEmpty ?? true)
    }
}
