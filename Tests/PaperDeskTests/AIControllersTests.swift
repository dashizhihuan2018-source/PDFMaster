import AppKit
import PDFKit
import XCTest
import PaperCore
@testable import PaperDesk

private final class AIControllerMockProtocol: URLProtocol {
    static var onStart: ((AIControllerMockProtocol) -> Void)?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() { Self.onStart?(self) }
    override func stopLoading() {}
    func finish(_ json: String) {
        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(json.utf8)); client?.urlProtocolDidFinishLoading(self)
    }
}

@MainActor final class AIControllersTests: XCTestCase {
    private func mockClient() -> DMXAIClient {
        let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [AIControllerMockProtocol.self]
        return DMXAIClient(configuration: config)
    }
    private func textDocument() -> PDFDocument {
        let view = NSTextView(frame: CGRect(x: 0, y: 0, width: 300, height: 300)); view.string = "Test PDF content"
        return PDFDocument(data: view.dataWithPDF(inside: view.bounds))!
    }
    func testStudioRequiresExplicitConsentBeforeReadingKey() {
        var keyReads = 0
        let controller = AIStudioController(keyProvider: { keyReads += 1; return nil }, onImageGenerated: { _, _ in })
        controller.promptField.string = "An image"
        XCTAssertEqual(controller.consentButton.state, .off)
        XCTAssertFalse(controller.startGeneration()); XCTAssertEqual(keyReads, 0)
        controller.close()
    }
    func testStudioMissingKeyPointsToSettingsWithoutRequest() {
        let controller = AIStudioController(keyProvider: { nil }, onImageGenerated: { _, _ in })
        controller.promptField.string = "An image"; controller.consentButton.state = .on
        XCTAssertFalse(controller.startGeneration()); XCTAssertFalse(controller.isRunning)
        XCTAssertTrue(controller.statusLabel.stringValue.contains("设置")); controller.close()
    }
    func testStudioRestoredTaskIsVisibleWithoutBackgroundQuery() {
        var keyReads = 0
        let controller = AIStudioController(keyProvider: { keyReads += 1; return nil }, onImageGenerated: { _, _ in }, restoredVideoTaskID: "task_previous")
        XCTAssertEqual(controller.taskIDField.stringValue, "task_previous"); XCTAssertEqual(keyReads, 0)
        XCTAssertFalse(controller.isRunning); controller.close()
    }
    func testCopilotOpensWithoutScanningDocumentOrReadingKey() {
        var scans = 0; var keyReads = 0
        let controller = PDFCopilotController(documentProvider: { scans += 1; return PDFDocument() }, currentPageProvider: { 0 }, keyProvider: { keyReads += 1; return nil }, onNavigate: { _ in })
        XCTAssertEqual(scans, 0); XCTAssertEqual(keyReads, 0)
        XCTAssertEqual(controller.consentButton.state, .off)
        XCTAssertFalse(controller.startGeneration()); XCTAssertEqual(scans, 0); XCTAssertEqual(keyReads, 0)
        controller.close()
    }
    func testCopilotScopeDefaultsToCurrentPageAndAllActionsAreAvailable() {
        let controller = PDFCopilotController(documentProvider: { nil }, currentPageProvider: { 0 }, keyProvider: { nil }, onNavigate: { _ in })
        XCTAssertEqual(controller.scopeChoice.indexOfSelectedItem, 0)
        XCTAssertEqual(controller.actionChoice.numberOfItems, 7)
        XCTAssertFalse(controller.answerView.isEditable)
        XCTAssertTrue(controller.modelField.stringValue.contains("doubao")); controller.close()
    }
    func testCopilotNoKeyDoesNotScanAndCloseRevokesConsent() {
        var scans = 0
        let controller = PDFCopilotController(documentProvider: { scans += 1; return nil }, currentPageProvider: { 0 }, keyProvider: { nil }, onNavigate: { _ in })
        controller.consentButton.state = .on
        XCTAssertFalse(controller.startGeneration()); XCTAssertEqual(scans, 0)
        controller.windowWillClose(Notification(name: NSWindow.willCloseNotification))
        XCTAssertEqual(controller.consentButton.state, .off); controller.close()
    }
    func testCopilotDiscardsAnswerWhenDocumentRevisionChangesDuringRequest() async throws {
        let doc = textDocument(); var revision = 0
        let controller = PDFCopilotController(documentProvider: { doc }, currentPageProvider: { 0 }, keyProvider: { "unit-test-key" }, onNavigate: { _ in }, revisionProvider: { revision }, client: mockClient())
        let started = expectation(description: "Started"); var pending: AIControllerMockProtocol?
        AIControllerMockProtocol.onStart = { pending = $0; started.fulfill() }
        defer { AIControllerMockProtocol.onStart = nil; controller.close() }
        controller.consentButton.state = .on; XCTAssertTrue(controller.startGeneration())
        await fulfillment(of: [started], timeout: 2); revision = 1
        try XCTUnwrap(pending).finish(#"{"choices":[{"message":{"content":"Should be discarded [p.1]"}}]}"#)
        for _ in 0..<100 where controller.isRunning { try await Task.sleep(nanoseconds: 2_000_000) }
        XCTAssertFalse(controller.isRunning); XCTAssertEqual(controller.answerView.string, "")
        XCTAssertTrue(controller.statusLabel.stringValue.contains("忽略"))
    }
    func testStudioPersistsOriginalAccountTaskAfterIdentitySwitchAndRejectsStaleUI() async throws {
        var identity = "original"; var savedID: String?
        let saved = expectation(description: "Original task saved")
        let controller = AIStudioController(keyProvider: { "unit-test-key" }, onImageGenerated: { _, _ in XCTFail("Video must not insert image") }, onVideoTaskSaved: { savedID = $0; saved.fulfill() }, sessionIdentityProvider: { identity }, client: mockClient())
        let started = expectation(description: "Started"); var pending: AIControllerMockProtocol?
        AIControllerMockProtocol.onStart = { pending = $0; started.fulfill() }
        defer { AIControllerMockProtocol.onStart = nil; controller.close() }
        controller.modeChoice.selectItem(at: 1); controller.promptField.string = "video"; controller.consentButton.state = .on
        XCTAssertTrue(controller.startGeneration()); await fulfillment(of: [started], timeout: 2)
        identity = "new-account"; try XCTUnwrap(pending).finish(#"{"id":"task_for_original"}"#)
        await fulfillment(of: [saved], timeout: 2)
        XCTAssertEqual(savedID, "task_for_original"); XCTAssertEqual(controller.taskIDField.stringValue, "")
        XCTAssertFalse(controller.isRunning)
    }
    func testVideoQueryCannotRebindOldAccountImageForCanvasInsertion() async throws {
        var identity = "image-account"; var inserted = 0
        let controller = AIStudioController(keyProvider: { "unit-test-key" }, onImageGenerated: { _, _ in inserted += 1 }, sessionIdentityProvider: { identity }, client: mockClient())
        let image = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        let encoded = image.representation(using: .png, properties: [:])!.base64EncodedString()
        AIControllerMockProtocol.onStart = { $0.finish("{\"data\":[{\"b64_json\":\"\(encoded)\"}]}") }
        defer { AIControllerMockProtocol.onStart = nil; controller.close() }
        controller.promptField.string = "image"; controller.consentButton.state = .on
        XCTAssertTrue(controller.startGeneration())
        for _ in 0..<100 where controller.isRunning { try await Task.sleep(nanoseconds: 2_000_000) }
        XCTAssertFalse(controller.isRunning)
        identity = "video-account"; controller.taskIDField.stringValue = "task_video"
        AIControllerMockProtocol.onStart = { $0.finish(#"{"output":[{"content":[{"text":"https://media.example.com/test.mp4"}]}]}"#) }
        _ = controller.perform(NSSelectorFromString("query"))
        for _ in 0..<100 where controller.isRunning { try await Task.sleep(nanoseconds: 2_000_000) }
        _ = controller.perform(NSSelectorFromString("insertImage"))
        XCTAssertEqual(inserted, 0)
    }
}
