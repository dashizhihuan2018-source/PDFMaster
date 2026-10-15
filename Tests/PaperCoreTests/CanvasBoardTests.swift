import Foundation
import CoreGraphics
import XCTest
@testable import PaperCore

@MainActor final class CanvasBoardTests: XCTestCase {
    func testErrorsExplainTheUserActionWithoutInternalObjectTerminology() {
        let messages: [(CanvasBoardError,String)] = [
            (.unsupportedFormat,"无法打开此文件，请选择 PDF Master 画布文件。"),
            (.invalidGeometry,"部分画布内容的位置或大小无效，无法完成此操作。"),
            (.invalidStyle,"颜色或描边宽度无效，请调整后重试。"),
            (.duplicateElement,"画布中的对象信息有冲突，请重新打开画布后重试。"),
            (.resourceLimit,"画布内容过多或附件过大，请减少对象数量或使用较小的文件。"),
            (.invalidAsset,"无法使用此附件，请重新选择有效的图片或 PDF 文件。"),
            (.localFileRequired,"请先将文件保存到这台 Mac，再选择该文件。"),
            (.invalidFile,"无法读取此画布文件，请检查文件是否完整且可以访问。")
        ]
        for (error,message) in messages { XCTAssertEqual(error.errorDescription,message) }
    }
    private func element(_ kind: CanvasElementKind = .rectangle) -> CanvasElement {
        CanvasElement(kind: kind, frame: CGRect(x: -20, y: 30, width: 80, height: 60))
    }

    func testRoundTripPreservesEmbeddedAssetsAndVectorPoints() throws {
        var card = element(.image)
        card.assetData = Data([0, 1, 2, 253, 254, 255]); card.assetName = "示例.png"
        var ink = element(.ink)
        ink.points = [CGPoint(x: -20, y: 30), CGPoint(x: 60, y: 90)]
        let board = CanvasBoard(title: "无限画布", elements: [card, ink])
        XCTAssertEqual(try CanvasBoard.decode(board.encoded()), board)
    }

    func testRejectsNonfiniteGeometryWithoutNormalization() {
        var item = element()
        item.frame.origin.x = .nan
        XCTAssertThrowsError(try CanvasBoard(elements: [item]).validated())
        item = element(.ink); item.points = [CGPoint(x: CGFloat.infinity, y: 1)]
        XCTAssertThrowsError(try CanvasBoard(elements: [item]).encoded())
    }

    func testRejectsNonpositiveAndOutOfBoundsFrames() {
        var item = element(); item.frame.size.width = 0
        XCTAssertThrowsError(try CanvasBoard(elements: [item]).validated())
        item.frame = CGRect(x: 1e9, y: 0, width: 20, height: 20)
        XCTAssertThrowsError(try CanvasBoard(elements: [item]).validated())
        item.frame = CGRect(x: 0, y: 0, width: -20, height: 20)
        XCTAssertThrowsError(try CanvasBoard(elements: [item]).validated())
    }

    func testRejectsBadStyleAndDuplicateIDs() {
        var item = element(); item.lineWidth = .nan
        XCTAssertThrowsError(try CanvasBoard(elements: [item]).validated())
        item.lineWidth = 0
        XCTAssertThrowsError(try CanvasBoard(elements: [item]).validated())
        item.lineWidth = 2; item.strokeHex = "javascript:alert(1)"
        XCTAssertThrowsError(try CanvasBoard(elements: [item]).validated())
        item = element()
        XCTAssertThrowsError(try CanvasBoard(elements: [item, item]).validated())
    }

    func testRejectsOversizedCollectionsAndMissingCardAsset() {
        let items = (0...10_000).map { _ in element() }
        XCTAssertThrowsError(try CanvasBoard(elements: items).validated())
        XCTAssertThrowsError(try CanvasBoard(elements: [element(.pdf)]).validated())
        var item = element(); item.assetData = Data([1])
        XCTAssertThrowsError(try CanvasBoard(elements: [item]).validated())
    }

    func testRejectsOversizedAssetBeforeEncoding() {
        var item = element(.image)
        item.assetData = Data(count: 50 * 1_024 * 1_024 + 1)
        XCTAssertThrowsError(try CanvasBoard(elements: [item]).encoded())
    }

    func testRejectsUnknownFormatAndTruncatedInput() {
        XCTAssertThrowsError(try CanvasBoard.decode(Data("{\"id\":\"abc\",\"elements\":[]}".utf8)))
        XCTAssertThrowsError(try CanvasBoard.decode(Data("not-json".utf8)))
        XCTAssertThrowsError(try CanvasBoard.decode(Data()))
        XCTAssertThrowsError(try CanvasBoard.decode(Data("{\"format\":\"AppleFreeform\",\"version\":1,\"board\":{}}".utf8)))
    }

    func testUnsupportedVersionIsRejected() throws {
        let data = try CanvasBoard().encoded()
        var json = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        json["version"] = 999
        XCTAssertThrowsError(try CanvasBoard.decode(JSONSerialization.data(withJSONObject: json)))
    }

    func testSavingAndLoadingRequiresLocalFileURL() throws {
        let remote = try XCTUnwrap(URL(string: "https://example.com/board.pdfmasterboard"))
        XCTAssertThrowsError(try CanvasBoard().save(to: remote))
        XCTAssertThrowsError(try CanvasBoard.load(from: remote))
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let url = folder.appendingPathComponent("本地.pdfmasterboard")
        let board = CanvasBoard(title: "本地", elements: [element(.sticky)])
        try board.save(to: url)
        XCTAssertEqual(try CanvasBoard.load(from: url), board)
        XCTAssertThrowsError(try CanvasBoard.load(from: folder))
    }

    func testViewportTransformsAndAnchoredZoom() {
        var viewport = CanvasViewport(scale: 2, offset: CGPoint(x: 50, y: 60))
        let world = CGPoint(x: -20, y: 100)
        let view = viewport.viewPoint(from: world)
        XCTAssertEqual(view, CGPoint(x: 10, y: 260))
        XCTAssertEqual(viewport.worldPoint(from: view), world)
        viewport.zoom(to: 4, anchor: view)
        XCTAssertEqual(viewport.worldPoint(from: view), world)
        viewport.pan(by: CGSize(width: 12, height: -7))
        XCTAssertEqual(viewport.viewPoint(from: world), CGPoint(x: 22, y: 253))
    }

    func testViewportClampAndInvalidInputAreSafe() {
        var viewport = CanvasViewport(scale: .nan, offset: CGPoint(x: CGFloat.infinity, y: 5))
        XCTAssertEqual(viewport.scale, 1); XCTAssertEqual(viewport.offset, CGPoint.zero)
        viewport.zoom(to: 99, anchor: .zero); XCTAssertEqual(viewport.scale, 8)
        viewport.zoom(to: 0.001, anchor: .zero); XCTAssertEqual(viewport.scale, 0.05)
        let previous = viewport
        viewport.zoom(to: .nan, anchor: .zero)
        viewport.pan(by: CGSize(width: CGFloat.infinity, height: 0))
        XCTAssertEqual(viewport.scale, previous.scale); XCTAssertEqual(viewport.offset, previous.offset)
    }

    func testViewportDirectMutationCannotIntroduceDivisionByZeroOrNaN() {
        var viewport = CanvasViewport()
        viewport.scale = 0
        XCTAssertEqual(viewport.scale, 0.05)
        XCTAssertTrue(viewport.worldPoint(from: CGPoint(x: 10, y: 10)).x.isFinite)
        viewport.scale = .nan
        XCTAssertTrue(viewport.scale.isFinite); XCTAssertGreaterThan(viewport.scale, 0)
        let previous = viewport.offset
        viewport.offset = CGPoint(x: CGFloat.nan, y: 3)
        XCTAssertEqual(viewport.offset, previous)
    }

    func testHistoryAssetRetentionIsBoundedRatherThanHolding100LargeBoards() throws {
        var card = element(.image)
        card.assetData = Data(count: 48 * 1_024 * 1_024)
        let session = CanvasSession(board: CanvasBoard(elements: [card]))
        for i in 0..<12 { try session.mutate { $0.title = "大附件版本\(i)" } }
        var undos = 0
        while session.canUndo { session.undo(); undos += 1 }
        XCTAssertLessThanOrEqual(undos, 5)
        XCTAssertGreaterThan(undos, 0)
        var redos = 0
        while session.canRedo { session.redo(); redos += 1 }
        XCTAssertEqual(redos, undos)
    }

    func testSessionMutationIsOneUndoableTransactionAndNoOpDoesNotNotify() throws {
        let session = CanvasSession()
        var notifications = 0; session.onChange = { notifications += 1 }
        XCTAssertFalse(session.isDirty); XCTAssertFalse(session.canUndo)
        try session.mutate { $0.elements.append(element()) }
        XCTAssertEqual(notifications, 1); XCTAssertTrue(session.isDirty); XCTAssertTrue(session.canUndo)
        try session.mutate { _ in }
        XCTAssertEqual(notifications, 1)
        session.undo()
        XCTAssertEqual(session.board.elements.count, 0); XCTAssertFalse(session.isDirty)
        XCTAssertTrue(session.canRedo)
        session.redo()
        XCTAssertEqual(session.board.elements.count, 1); XCTAssertTrue(session.isDirty)
    }

    func testInvalidMutationDoesNotAlterBoardOrHistory() throws {
        let session = CanvasSession()
        let original = session.board
        var notifications = 0; session.onChange = { notifications += 1 }
        XCTAssertThrowsError(try session.mutate { $0.elements.append(CanvasElement(kind: .rectangle, frame: .zero)) })
        XCTAssertEqual(session.board, original)
        XCTAssertFalse(session.canUndo); XCTAssertFalse(session.isDirty); XCTAssertEqual(notifications, 0)
    }

    func testSavedSnapshotTracksUndoAndRedo() throws {
        let session = CanvasSession()
        try session.mutate { $0.title = "第一版" }
        session.markSaved(); XCTAssertFalse(session.isDirty)
        try session.mutate { $0.title = "第二版" }
        XCTAssertTrue(session.isDirty)
        session.undo(); XCTAssertFalse(session.isDirty)
        session.redo(); XCTAssertTrue(session.isDirty)
        session.undo()
        try session.mutate { $0.title = "新分支" }
        XCTAssertFalse(session.canRedo)
    }

    func testReplaceBoardRejectsInvalidAndClearsHistoryForValidLoad() throws {
        let session = CanvasSession()
        try session.mutate { $0.title = "旧" }
        let old = session.board
        XCTAssertThrowsError(try session.replaceBoard(CanvasBoard(elements: [CanvasElement(kind: .ellipse, frame: .zero)])))
        XCTAssertEqual(session.board, old); XCTAssertTrue(session.canUndo)
        let new = CanvasBoard(title: "已载入")
        try session.replaceBoard(new)
        XCTAssertEqual(session.board, new); XCTAssertFalse(session.canUndo)
        XCTAssertFalse(session.canRedo); XCTAssertFalse(session.isDirty)
    }

    func testHistoryIsBoundedAndSaveFailureDoesNotMarkClean() throws {
        let session = CanvasSession()
        for i in 0..<110 { try session.mutate { $0.title = "第\(i)次" } }
        var undoCount = 0
        while session.canUndo { session.undo(); undoCount += 1 }
        XCTAssertEqual(undoCount, 100)
        try session.mutate { $0.title = "待保存" }
        XCTAssertThrowsError(try session.save(to: URL(string: "https://example.com/board")!))
        XCTAssertTrue(session.isDirty)
    }

    func testSessionSaveMarksCleanAndPreservesUndo() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let url = folder.appendingPathComponent("board.pdfmasterboard")
        let session = CanvasSession()
        try session.mutate { $0.title = "保存版本" }
        try session.save(to: url)
        XCTAssertEqual(try CanvasBoard.load(from: url), session.board)
        XCTAssertFalse(session.isDirty); XCTAssertTrue(session.canUndo)
        session.undo(); XCTAssertTrue(session.isDirty)
        session.redo(); XCTAssertFalse(session.isDirty)
    }
}
