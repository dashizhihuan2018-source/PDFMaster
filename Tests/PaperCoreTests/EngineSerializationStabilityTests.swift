import AppKit
import PDFKit
import XCTest
@testable import PaperCore

@MainActor
final class EngineSerializationStabilityTests: XCTestCase {
    func testUnchangedEngineSnapshotRemainsValidAcrossSerializationClockTick() throws {
        let workspace = PDFWorkspace()
        try workspace.createBlank()
        let originalCreation = workspace.document?.documentAttributes?[PDFDocumentAttribute.creationDateAttribute] as? Date
        let originalModification = workspace.document?.documentAttributes?[PDFDocumentAttribute.modificationDateAttribute] as? Date
        let first = try workspace.engineInput()
        Thread.sleep(forTimeInterval: 1.2)
        let second = try workspace.engineInput()
        if first.data != second.data {
            let a = String(decoding: first.data, as: UTF8.self).split(separator: "\n")
            let b = String(decoding: second.data, as: UTF8.self).split(separator: "\n")
            for (index, pair) in zip(a, b).enumerated() where pair.0 != pair.1 {
                print("Serialization difference line \(index): \(pair.0.prefix(200)) => \(pair.1.prefix(200))")
            }
        }
        XCTAssertNoThrow(try workspace.validateEngineInput(first))
        // Digest normalization never changes the user's live metadata.
        XCTAssertEqual(workspace.document?.documentAttributes?[PDFDocumentAttribute.creationDateAttribute] as? Date, originalCreation)
        XCTAssertEqual(workspace.document?.documentAttributes?[PDFDocumentAttribute.modificationDateAttribute] as? Date, originalModification)
        XCTAssertNoThrow(try workspace.acceptEngineResult(first.data, for: first))
    }

    func testDirectAnnotationAndMetadataChangesStillRejectOldEngineInput() throws {
        let workspace = PDFWorkspace()
        try workspace.createBlank()
        let original = try workspace.engineInput()
        let annotation = PDFAnnotation(bounds: CGRect(x: 30, y: 30, width: 60, height: 20), forType: .freeText, withProperties: nil)
        annotation.contents = "New direct content"
        workspace.document?.page(at: 0)?.addAnnotation(annotation)
        XCTAssertThrowsError(try workspace.validateEngineInput(original)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .staleEngineResult)
        }

        try workspace.createBlank()
        let beforeMetadata = try workspace.engineInput()
        workspace.document?.documentAttributes = [PDFDocumentAttribute.titleAttribute: "Changed directly"]
        XCTAssertThrowsError(try workspace.validateEngineInput(beforeMetadata)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .staleEngineResult)
        }

        try workspace.createBlank()
        let beforeDate = try workspace.engineInput()
        workspace.document?.documentAttributes = [PDFDocumentAttribute.creationDateAttribute: Date(timeIntervalSince1970: 1234)]
        XCTAssertThrowsError(try workspace.validateEngineInput(beforeDate)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .staleEngineResult)
        }
    }

    func testDateLookingTextInRealPageContentStreamStillDetectsDirectMutation() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = directory.appendingPathComponent("date-text-fixture.pdf")
        let original = pageContentFixture(date: "20261012174000")
        XCTAssertNotNil(original.range(of: Data("stream\nBT /F1 12 Tf 40 700 Td (/CreationDate".utf8)))
        try original.write(to: source)
        let workspace = PDFWorkspace()
        try workspace.open(source)
        let snapshot = try workspace.engineInput()
        Thread.sleep(forTimeInterval: 1.2)
        XCTAssertNoThrow(try workspace.validateEngineInput(snapshot))
        let changed = try XCTUnwrap(PDFDocument(data: pageContentFixture(date: "20261012174001")))
        let page = try XCTUnwrap(changed.page(at: 0)?.copy() as? PDFPage)
        workspace.document?.removePage(at: 0)
        workspace.document?.insert(page, at: 0)
        XCTAssertEqual(workspace.revision, 0)
        XCTAssertThrowsError(try workspace.validateEngineInput(snapshot)) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .staleEngineResult)
        }
    }

    func testUnknownLiveMetadataTypeFailsClosedBeforeSerialization() throws {
        let workspace = PDFWorkspace()
        try workspace.createBlank()
        workspace.document?.documentAttributes = [PDFDocumentAttribute.titleAttribute: NSColor.red]
        XCTAssertThrowsError(try workspace.engineInput()) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .unreadableDocument)
        }
    }

    func testDeepOrOversizedLiveMetadataFailsClosedBeforeSerialization() throws {
        let workspace = PDFWorkspace()
        try workspace.createBlank()
        var nested: Any = "leaf"
        for _ in 0..<40 { nested = [nested] }
        workspace.document?.documentAttributes = [PDFDocumentAttribute.keywordsAttribute: nested]
        XCTAssertThrowsError(try workspace.engineInput()) {
            XCTAssertEqual($0 as? PDFWorkspaceError, .unreadableDocument)
        }
        workspace.document?.documentAttributes = [PDFDocumentAttribute.titleAttribute: String(repeating: "x", count: 4 * 1024 * 1024 + 1)]
        XCTAssertThrowsError(try workspace.engineInput())
        workspace.document?.documentAttributes = [PDFDocumentAttribute.keywordsAttribute: Array(repeating: "x", count: 10001)]
        XCTAssertThrowsError(try workspace.engineInput())
        workspace.document?.documentAttributes = [PDFDocumentAttribute.titleAttribute: Data(repeating: 0, count: 4 * 1024 * 1024 + 1)]
        XCTAssertThrowsError(try workspace.engineInput())
    }

    private func pageContentFixture(date: String) -> Data {
        let content = "BT /F1 12 Tf 40 700 Td (/CreationDate \\(D:\(date)Z00'00'\\) /ModDate \\(D:\(date)Z00'00'\\)) Tj ET\n"
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            "<< /Length \(content.utf8.count) >>\nstream\n\(content)endstream"
        ]
        var data = Data("%PDF-1.4\n".utf8)
        var offsets: [Int] = [0]
        for (index, object) in objects.enumerated() {
            offsets.append(data.count)
            data.append(Data("\(index + 1) 0 obj\n\(object)\nendobj\n".utf8))
        }
        let xref = data.count
        data.append(Data("xref\n0 \(objects.count + 1)\n0000000000 65535 f \n".utf8))
        for offset in offsets.dropFirst() { data.append(Data(String(format: "%010d 00000 n \n", offset).utf8)) }
        data.append(Data("trailer\n<< /Size \(objects.count + 1) /Root 1 0 R >>\nstartxref\n\(xref)\n%%EOF\n".utf8))
        return data
    }
}
