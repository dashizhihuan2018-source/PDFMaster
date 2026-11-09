import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor final class PDFSignatureTests: XCTestCase {
    private func image() throws -> SignatureImage {
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 8, bitsPerPixel: 32)!
        let bytes: [UInt8] = [0,0,0,0, 255,0,0,255, 0,0,255,255, 0,128,0,128]
        let storage = try XCTUnwrap(bitmap.bitmapData)
        for i in 0..<bytes.count { storage[i] = bytes[i] }
        return try SignatureImage(pngData: XCTUnwrap(bitmap.representation(using: .png, properties: [:])))
    }

    func testStampPersistsAndUndoRedoAndRemove() throws {
        for rotation in [0, 90, 180, 270] {
            let workspace = PDFWorkspace(); try workspace.createBlank()
            if rotation != 0 { try workspace.rotate(page: 0, degrees: rotation) }
            try workspace.addImageSignature(image(), page: 0, bounds: CGRect(x: 50, y: 60, width: 100, height: 80))
            let saved = try XCTUnwrap(workspace.document?.dataRepresentation())
            let reopened = try XCTUnwrap(PDFDocument(data: saved))
            let annotation = try XCTUnwrap(reopened.page(at: 0)?.annotations.first)
            XCTAssertEqual(annotation.type, "Stamp")
            let provider = try XCTUnwrap(CGDataProvider(data: saved as CFData))
            let cg = try XCTUnwrap(CGPDFDocument(provider))
            let page = try XCTUnwrap(cg.page(at: 1))
            var annots: CGPDFArrayRef?; XCTAssertTrue(CGPDFDictionaryGetArray(try XCTUnwrap(page.dictionary), "Annots", &annots))
            var dictionary: CGPDFDictionaryRef?; XCTAssertTrue(CGPDFArrayGetDictionary(try XCTUnwrap(annots), 0, &dictionary))
            var appearance: CGPDFDictionaryRef?; XCTAssertTrue(CGPDFDictionaryGetDictionary(try XCTUnwrap(dictionary), "AP", &appearance))
            XCTAssertEqual(reopened.page(at: 0)?.annotations.count, 1)
            workspace.undo(); XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.count, 0)
            workspace.redo(); XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.count, 1)
            let url = FileManager.default.temporaryDirectory.appendingPathComponent("signature-\(UUID().uuidString).pdf")
            defer { try? FileManager.default.removeItem(at: url) }
            try workspace.saveCopy(to: url)
            XCTAssertEqual(PDFDocument(url: url)?.page(at: 0)?.annotations.count, 1)
            try workspace.removeAnnotation(page: 0, index: 0)
            XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.count, 0)
        }
    }

    func testInvalidStampBoundsAndMultistrokeInk() throws {
        let workspace = PDFWorkspace(); try workspace.createBlank()
        XCTAssertThrowsError(try workspace.addImageSignature(image(), page: 0, bounds: CGRect(x: -10, y: 0, width: 100, height: 80)))
        let strokes = [[CGPoint(x: 40, y: 50), CGPoint(x: 60, y: 70)], [CGPoint(x: 70, y: 80), CGPoint(x: 90, y: 60)]]
        try workspace.addSignatureInk(strokes: strokes, page: 0, color: .black, width: 2)
        let doc = try XCTUnwrap(PDFDocument(data: XCTUnwrap(workspace.document?.dataRepresentation())))
        let annotations = try XCTUnwrap(doc.page(at: 0)?.annotations)
        XCTAssertEqual(annotations.count, 1)
        XCTAssertEqual(annotations.first?.paths?.count, 2)
    }

    func testAsymmetricImageRendersUprightAfterReopeningEveryRotation() throws {
        var reference: [[CGFloat]]?
        for rotation in [0, 90, 180, 270] {
            let workspace = PDFWorkspace(); try workspace.createBlank()
            let background = NSImage(size: CGSize(width: 200, height: 200))
            background.lockFocus(); NSColor.yellow.setFill(); NSRect(x: 0, y: 0, width: 200, height: 200).fill(); background.unlockFocus()
            let document = try XCTUnwrap(workspace.document)
            document.removePage(at: 0); document.insert(try XCTUnwrap(PDFPage(image: background)), at: 0)
            workspace.document!.page(at: 0)!.setBounds(CGRect(x: 0, y: 0, width: 200, height: 200), for: .mediaBox)
            workspace.document!.page(at: 0)!.setBounds(CGRect(x: 0, y: 0, width: 200, height: 200), for: .cropBox)
            if rotation != 0 { try workspace.rotate(page: 0, degrees: rotation) }
            try workspace.addImageSignature(image(), page: 0, bounds: CGRect(x: 50, y: 50, width: 100, height: 100))
            let reopened = try XCTUnwrap(PDFDocument(data: XCTUnwrap(workspace.document?.dataRepresentation())))
            let thumbnail = try XCTUnwrap(reopened.page(at: 0)).thumbnail(of: CGSize(width: 200, height: 200), for: .cropBox)
            let bitmap = try XCTUnwrap(NSBitmapImageRep(data: XCTUnwrap(thumbnail.tiffRepresentation)))
            let colors = try [(75,75), (125,75), (75,125), (125,125)].map { x, y -> [CGFloat] in
                let color = try XCTUnwrap(bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB))
                return [color.redComponent, color.greenComponent, color.blueComponent]
            }
            if let reference { for (expected, actual) in zip(reference.flatMap { $0 }, colors.flatMap { $0 }) { XCTAssertEqual(actual, expected, accuracy: 0.08, "rotation \(rotation): \(colors)") } }
            else {
                reference = colors
                // Transparent corner shows yellow; opaque red and blue remain distinct;
                // half-transparent green blends with the yellow page, not black.
                XCTAssertGreaterThan(colors[0][0], 0.9); XCTAssertGreaterThan(colors[0][1], 0.9)
                XCTAssertGreaterThan(colors[1][0], 0.8); XCTAssertLessThan(colors[1][1], 0.4)
                XCTAssertGreaterThan(colors[2][2], 0.8); XCTAssertLessThan(colors[2][0], 0.3)
                XCTAssertGreaterThan(colors[3][0], 0.4); XCTAssertLessThan(colors[3][0], 0.9)
                XCTAssertGreaterThan(colors[3][1], 0.8)
            }
        }
    }

    func testCommentingPermissionDeniesBothSignaturesWithoutMutation() throws {
        let source = PDFWorkspace(); try source.createBlank()
        let allowed: [PDFAccessPermissions] = [.allowsLowQualityPrinting, .allowsHighQualityPrinting, .allowsDocumentAssembly, .allowsContentCopying, .allowsContentAccessibility, .allowsFormFieldEntry]
        let permissions = allowed.reduce(UInt(0)) { $0 | $1.rawValue }
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("signature-restricted-\(UUID().uuidString).pdf")
        defer { try? FileManager.default.removeItem(at: url) }
        XCTAssertTrue(try XCTUnwrap(source.document).write(to: url, withOptions: [.ownerPasswordOption: "owner-only", .accessPermissionsOption: NSNumber(value: permissions)]))
        let workspace = PDFWorkspace(); try workspace.open(url)
        XCTAssertThrowsError(try workspace.addImageSignature(image(), page: 0, bounds: CGRect(x: 30, y: 30, width: 50, height: 50))) { XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied) }
        XCTAssertThrowsError(try workspace.addSignatureInk(strokes: [[CGPoint(x: 30, y: 30), CGPoint(x: 50, y: 50)]], page: 0, color: .black, width: 2)) { XCTAssertEqual($0 as? PDFWorkspaceError, .permissionDenied) }
        XCTAssertEqual(workspace.revision, 0)
        XCTAssertEqual(workspace.document?.page(at: 0)?.annotations.count, 0)
    }
}
