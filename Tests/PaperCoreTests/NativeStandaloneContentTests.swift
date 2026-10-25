import Foundation
import PDFKit
import XCTest
import CryptoKit
@testable import PaperCore

final class NativeStandaloneContentTests: XCTestCase {
    private lazy var publicFixture = try! PublicContentFixture()
    private func fixture(active: Bool = false, stream override: String? = nil) -> Data {
        let stream = override ?? "BT /F1 14 Tf 1 0 0 1 40 150 Tm (Original target) Tj ET\nBT /F1 14 Tf 1 0 0 1 40 100 Tm (Neighbor preserved) Tj ET"
        let objects = ["<< /Type /Catalog /Pages 2 0 R \(active ? "/OpenAction [3 0 R /Fit]" : "") >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>", "<< /Length \(stream.utf8.count) >>\nstream\n\(stream)\nendstream", "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
        var text = "%PDF-1.7\n", offsets = [0]
        for (index, object) in objects.enumerated() { offsets.append(text.utf8.count); text += "\(index + 1) 0 obj\n\(object)\nendobj\n" }
        let start = text.utf8.count
        text += "xref\n0 \(objects.count + 1)\n0000000000 65535 f \n" + offsets.dropFirst().map { String(format: "%010d 00000 n \n", $0) }.joined()
        text += "trailer\n<< /Size \(objects.count + 1) /Root 1 0 R >>\nstartxref\n\(start)\n%%EOF\n"
        return Data(text.utf8)
    }
    private func withFiles(_ body: (URL, URL) throws -> Void) throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("NativeStandalone-\(UUID())")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: dir) }
        let input = dir.appendingPathComponent("input.pdf")
        try fixture().write(to: input)
        try body(input, dir.appendingPathComponent("output.pdf"))
    }
    private var engine: ContentEditEngine { ContentEditEngine(python: URL(fileURLWithPath: "/does-not-exist/python"), script: URL(fileURLWithPath: "/does-not-exist/engine.py")) }
    func testInspectWorksWithoutPythonOrScriptAndReturnsRealText() throws {
        try withFiles { input, _ in
            let page = try engine.inspect(input, page: 0, cancelled: { false })
            XCTAssertEqual(page.spans.map(\.text), ["Original target", "Neighbor preserved"])
            XCTAssertEqual(page.width, 400)
            XCTAssertTrue(page.spans.allSatisfy { $0.bbox.count == 4 && $0.origin.count == 2 })
        }
    }
    func testReplacementActuallyRemovesOldTextAndPreservesOtherTextAndSource() throws {
        try withFiles { input, output in
            let before = try Data(contentsOf: input)
            let page = try engine.inspect(input, page: 0, cancelled: { false })
            var request = ContentEditRequest(operation: "replace-text", page: 0)
            request.id = try XCTUnwrap(page.spans.first).id; request.text = "Edited Ω"
            _ = try engine.apply(input, request: request, output: output, cancelled: { false })
            let result = try XCTUnwrap(PDFDocument(url: output)?.page(at: 0))
            XCTAssertTrue(result.string?.contains("Edited Ω") == true)
            XCTAssertTrue(result.string?.contains("Neighbor preserved") == true)
            XCTAssertFalse(result.string?.contains("Original target") == true)
            XCTAssertFalse(result.annotations.contains { ["Square", "FreeText", "Redact"].contains($0.type ?? "") })
            XCTAssertEqual(try Data(contentsOf: input), before)
        }
    }
    func testCancellationAndActiveContentNeverCreateOutput() throws {
        try withFiles { input, output in
            XCTAssertThrowsError(try engine.inspect(input, page: 0, cancelled: { true }))
            try fixture(active: true).write(to: input)
            XCTAssertThrowsError(try engine.inspect(input, page: 0, cancelled: { false }))
            XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
        }
    }
    func testClippedOrSpeciallySpacedTextIsExplicitlyRefusedWithoutOutput() throws {
        for stream in ["BT /F1 14 Tf 1 0 0 1 40 150 Tm 2 Tc (Original target) Tj ET", "q 0 0 400 200 re W n BT /F1 14 Tf 1 0 0 1 40 150 Tm (Original target) Tj ET Q"] {
            try withFiles { input,output in
                let original = fixture(stream:stream); try original.write(to:input)
                let page = try engine.inspect(input,page:0,cancelled:{false})
                var request = ContentEditRequest(operation:"replace-text",page:0)
                request.id = try XCTUnwrap(page.spans.first).id; request.text = "Edited"
                XCTAssertThrowsError(try engine.apply(input,request:request,output:output,cancelled:{false}))
                XCTAssertFalse(FileManager.default.fileExists(atPath:output.path)); XCTAssertEqual(try Data(contentsOf:input),original)
            }
        }
    }
    func testActualTextMarkedContentIsRejectedInsteadOfLeavingHiddenReplacementText() throws {
        try withFiles { input,output in
            let original = fixture(stream:"/Span << /ActualText (Secret alternative) >> BDC BT /F1 14 Tf 1 0 0 1 40 150 Tm (Original target) Tj ET EMC")
            try original.write(to:input)
            XCTAssertThrowsError(try engine.inspect(input,page:0,cancelled:{false}))
            XCTAssertEqual(try Data(contentsOf:input),original); XCTAssertFalse(FileManager.default.fileExists(atPath:output.path))
        }
    }
    func testInsertUpdateDeleteLinkDoesNotNeedExternalRuntime() throws {
        try withFiles { input, output in
            var insert = ContentEditRequest(operation: "insert-link", page: 0)
            insert.bbox = [40, 10, 150, 30]; insert.uri = "https://example.com/one"
            _ = try engine.apply(input, request: insert, output: output, cancelled: { false })
            let links = try engine.inspect(output, page: 0, cancelled: { false }).links
            XCTAssertEqual(links.map(\.uri), ["https://example.com/one"])
            let next = output.deletingLastPathComponent().appendingPathComponent("next.pdf")
            var update = ContentEditRequest(operation: "update-link", page: 0)
            update.id = try XCTUnwrap(links.first).id; update.uri = "https://example.com/two"
            _ = try engine.apply(output, request: update, output: next, cancelled: { false })
            let final = output.deletingLastPathComponent().appendingPathComponent("final.pdf")
            var remove = ContentEditRequest(operation: "delete-link", page: 0)
            remove.id = try XCTUnwrap(engine.inspect(next, page: 0, cancelled: { false }).links.first).id
            _ = try engine.apply(next, request: remove, output: final, cancelled: { false })
            XCTAssertTrue(try engine.inspect(final, page: 0, cancelled: { false }).links.isEmpty)
        }
    }
    func testAddingLinkPreservesAnExistingIndirectAnnotationArray() throws {
        try withFiles { source,output in
            let graph = try NativeContentGraph(input:source,qpdf:URL(fileURLWithPath: ProcessInfo.processInfo.environment["PDFMASTER_TEST_QPDF"] ?? "/unavailable/qpdf"),cancelled:{false})
            var page = try graph.pageDictionary(at:0).1; page["/Annots"] = "6 0 R"
            graph.setObject("3 0 R",value:page); graph.updates["obj:6 0 R"] = ["value":["7 0 R"]]
            graph.setObject("7 0 R",value:["/Type":"/Annot","/Subtype":"/Link","/Rect":[10,10,50,25],"/A":["/S":"/URI","/URI":"u:https://example.com/existing"]])
            let input = output.deletingLastPathComponent().appendingPathComponent("indirect.pdf")
            try graph.rewrite(output:input)
            var request = ContentEditRequest(operation:"insert-link",page:0); request.bbox = [100,100,200,120]; request.uri = "https://example.com/new"
            _ = try engine.apply(input,request:request,output:output,cancelled:{false})
            XCTAssertEqual(Set(try engine.inspect(output,page:0,cancelled:{false}).links.map(\.uri)),Set(["https://example.com/existing","https://example.com/new"]))
        }
    }
    func testOriginalFixtureImageReplacementAndRealRedactionAreNative() throws {
        let source = publicFixture.source
        XCTAssertTrue(FileManager.default.fileExists(atPath:source.path))
        let original = try Data(contentsOf:source)
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("NativeImageRedact-\(UUID())")
        try FileManager.default.createDirectory(at:dir,withIntermediateDirectories:false)
        defer { try? FileManager.default.removeItem(at:dir) }
        let input = dir.appendingPathComponent("input.pdf"), output = dir.appendingPathComponent("image.pdf"), redacted = dir.appendingPathComponent("redacted.pdf")
        try original.write(to:input)
        var page = try engine.inspect(input,page:0,cancelled:{false})
        var request = ContentEditRequest(operation:"replace-image",page:0)
        request.id = try XCTUnwrap(page.images.first).id
        request.image_path = publicFixture.replacement.path
        _ = try engine.apply(input,request:request,output:output,cancelled:{false})
        let doc = try XCTUnwrap(PDFDocument(url:output)); XCTAssertEqual(doc.pageCount,20)
        XCTAssertEqual(doc.page(at:0)?.string,PDFDocument(url:input)?.page(at:0)?.string)
        page = try engine.inspect(output,page:0,cancelled:{false})
        let target = try XCTUnwrap(page.spans.first { $0.text == "Original target" })
        request = ContentEditRequest(operation:"redact",page:0); request.bbox = target.bbox
        _ = try engine.apply(output,request:request,output:redacted,cancelled:{false})
        let result = try XCTUnwrap(PDFDocument(url:redacted)?.page(at:0))
        XCTAssertFalse(result.string!.contains("Original target")); XCTAssertTrue(result.string!.contains("Neighbor preserved"))
        XCTAssertFalse(result.annotations.contains { ["Square","Redact"].contains($0.type ?? "") })
        XCTAssertEqual(try Data(contentsOf:source),original)
    }
    func testDeletingImageRemovesItsOriginalPixelStreamNotOnlyItsPaintOperator() throws {
        let source = publicFixture.source
        try XCTSkipUnless(FileManager.default.fileExists(atPath:source.path))
        let executable = URL(fileURLWithPath: ProcessInfo.processInfo.environment["PDFMASTER_TEST_QPDF"] ?? "/unavailable/qpdf")
        let graph = try NativeContentGraph(input:source,qpdf:executable,cancelled:{false})
        let image = try XCTUnwrap(graph.scan(page:0).first { $0.kind == .image })
        let originalObject = try XCTUnwrap(graph.objects["obj:" + image.reference!] as? [String:Any])
        let originalStream = try XCTUnwrap((originalObject["stream"] as? [String:Any])?["data"] as? String)
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("NativeImageDelete-\(UUID())")
        try FileManager.default.createDirectory(at:dir,withIntermediateDirectories:false)
        defer { try? FileManager.default.removeItem(at:dir) }
        let output = dir.appendingPathComponent("deleted.pdf")
        let page = try engine.inspect(source,page:0,cancelled:{false})
        var request = ContentEditRequest(operation:"redact",page:0); request.bbox = try XCTUnwrap(page.images.first).bbox
        _ = try engine.apply(source,request:request,output:output,cancelled:{false})
        let after = try NativeContentGraph(input:output,qpdf:executable,cancelled:{false})
        XCTAssertFalse(after.objects.values.contains { object in
            guard let value = object as? [String:Any], let stream = value["stream"] as? [String:Any] else { return false }
            return stream["data"] as? String == originalStream
        }, "The original image bytes must not remain extractable in an unused resource")
        XCTAssertTrue(try engine.inspect(output,page:0,cancelled:{false}).images.isEmpty)
    }
    func testSelfAuthoredFivePageChineseImageDocumentEditsAndReopensWithoutChangingOriginal() throws {
        let sample = try PublicContentFixture(pageCount: 5, title: "示例古建", imagePageCount: 4)
        let source = sample.source
        let original = try Data(contentsOf:source)
        func hash(_ data:Data) -> String { SHA256.hash(data:data).map { String(format:"%02x",$0) }.joined() }
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("PublicChinese-Native-\(UUID())")
        try FileManager.default.createDirectory(at:dir,withIntermediateDirectories:false)
        defer { try? FileManager.default.removeItem(at:dir) }
        let input = dir.appendingPathComponent("input.pdf"), output = dir.appendingPathComponent("edited.pdf")
        try original.write(to:input)
        let before = try XCTUnwrap(PDFDocument(url:input)); XCTAssertEqual(before.pageCount,5)
        var pages:[ContentPage] = []
        for index in 0..<5 { pages.append(try engine.inspect(input,page:index,cancelled:{false})) }
        XCTAssertEqual(pages.map(\.images.count),[1,1,1,1,0])
        XCTAssertTrue(pages[0].spans.contains { $0.text == "示例古建" })
        let target = try XCTUnwrap(pages[0].spans.first { $0.text == "示例古建" })
        var request = ContentEditRequest(operation:"replace-text",page:0)
        request.id = target.id; request.text = "阅读示例"
        _ = try engine.apply(input,request:request,output:output,cancelled:{false})
        let reopened = try XCTUnwrap(PDFDocument(url:output)); XCTAssertEqual(reopened.pageCount,5)
        XCTAssertFalse(reopened.page(at:0)!.string!.contains("示例古建"))
        XCTAssertTrue(reopened.page(at:0)!.string!.contains("阅读示例"))
        for index in 1..<5 { XCTAssertEqual(reopened.page(at:index)?.string,before.page(at:index)?.string) }
        let after = try engine.inspect(output,page:0,cancelled:{false})
        XCTAssertEqual(after.images.map(\.bbox),pages[0].images.map(\.bbox))
        let executable = URL(fileURLWithPath: ProcessInfo.processInfo.environment["PDFMASTER_TEST_QPDF"] ?? "/unavailable/qpdf")
        func imageDigests(_ url:URL) throws -> [String] {
            let graph = try NativeContentGraph(input:url,qpdf:executable,cancelled:{false})
            return graph.objects.values.compactMap { value in
                guard let object = value as? [String:Any], let stream = object["stream"] as? [String:Any],
                      let dict = stream["dict"] as? [String:Any], dict["/Subtype"] as? String == "/Image", let encoded = stream["data"] as? String,
                      let bytes = Data(base64Encoded:encoded) else { return nil }
                return hash(bytes)
            }.sorted()
        }
        let originalImages = try imageDigests(input)
        XCTAssertEqual(originalImages.count,4); XCTAssertEqual(try imageDigests(output),originalImages,"All four self-authored image streams must remain byte-identical")
        let link = dir.appendingPathComponent("linked.pdf")
        request = ContentEditRequest(operation:"insert-link",page:4); request.bbox = [40,40,200,65]; request.uri = "https://example.com/pdf-master"
        _ = try engine.apply(output,request:request,output:link,cancelled:{false})
        XCTAssertEqual(try engine.inspect(link,page:4,cancelled:{false}).links.last?.uri,"https://example.com/pdf-master")
        XCTAssertEqual(hash(try Data(contentsOf:source)),hash(original)); XCTAssertEqual(try Data(contentsOf:input),original)
    }
}
