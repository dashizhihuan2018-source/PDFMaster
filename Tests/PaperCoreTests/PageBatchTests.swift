import XCTest
import PDFKit
@testable import PaperCore

@MainActor final class PageBatchTests: XCTestCase {
    func testBatchRotationAndClipboardAreAtomicAndDoNotChangeOtherPages() throws {
        let c = PDFWorkspace(); try c.createBlank(); try c.insertBlank(at: 1); try c.insertBlank(at: 2)
        try c.rotate(pages: [1, 2], degrees: 90)
        XCTAssertEqual((0..<3).map { c.document!.page(at: $0)!.rotation }, [0, 90, 90])
        c.undo(); XCTAssertEqual((0..<3).map { c.document!.page(at: $0)!.rotation }, [0, 0, 0])
        let data = try c.copyPages([1, 2])
        XCTAssertEqual(PDFDocument(data: data)?.pageCount, 2)
        XCTAssertEqual(c.document?.pageCount, 3)
        try c.pastePages(data, at: 1)
        XCTAssertEqual(c.document?.pageCount, 5)
        c.undo(); XCTAssertEqual(c.document?.pageCount, 3)
    }
}
