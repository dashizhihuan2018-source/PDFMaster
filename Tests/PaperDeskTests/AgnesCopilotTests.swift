import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

private final class AgnesCopilotProtocol: URLProtocol {
    static var bodies: [[String: Any]] = []
    static var hosts: [String] = []
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        var bytes = request.httpBody ?? Data()
        if let s = request.httpBodyStream { s.open(); defer { s.close() }; var b = [UInt8](repeating: 0, count: 4096); while s.hasBytesAvailable { let n = s.read(&b, maxLength: b.count); if n <= 0 { break }; bytes.append(contentsOf: b.prefix(n)) } }
        Self.bodies.append((try? JSONSerialization.jsonObject(with: bytes)) as? [String: Any] ?? [:]); Self.hosts.append(request.url!.host!)
        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(#"{"choices":[{"message":{"content":"Synthetic summary [p.1]"}}]}"#.utf8)); client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
@MainActor final class AgnesCopilotTests: XCTestCase {
    private func document() -> PDFDocument {
        let text = NSTextView(frame: NSRect(x: 0, y: 0, width: 300, height: 300)); text.string = "Synthetic public fixture: launch date is November 5."
        return PDFDocument(data: text.dataWithPDF(inside: text.bounds))!
    }
    private func controller(agnesKey: String? = "agnes-test", reads: (() -> Void)? = nil) -> PDFCopilotController {
        let doc = document(); let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [AgnesCopilotProtocol.self]
        return PDFCopilotController(documentProvider: { doc }, currentPageProvider: { 0 }, keyProvider: { XCTFail("Agnes must never read DMX key"); return "dmx-test" }, onNavigate: { _ in }, client: DMXAIClient(configuration: config), agnesKeyProvider: { reads?(); return agnesKey }, initialProvider: .agnes)
    }
    func testAgnesIsExplicitAndMissingConsentNeverReadsKey() {
        var reads = 0; let c = controller { reads += 1 }; defer { c.close() }
        XCTAssertEqual(c.selectedProvider, .agnes); XCTAssertEqual(c.modelField.stringValue, "agnes-3.0-flash")
        XCTAssertTrue(c.consentButton.title.contains("Agnes")); XCTAssertEqual(c.consentButton.state, .off)
        XCTAssertFalse(c.startGeneration()); XCTAssertEqual(reads, 0)
    }
    func testDeepSeekRequiresConsentAndUsesOnlyItsOwnKeyAndRichText() async throws {
        AgnesCopilotProtocol.hosts = []; AgnesCopilotProtocol.bodies = []
        let doc = document(); let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [AgnesCopilotProtocol.self]
        var reads = 0
        let c = PDFCopilotController(documentProvider: { doc }, currentPageProvider: { 0 }, keyProvider: { XCTFail("No DMX fallback"); return nil }, onNavigate: { _ in }, client: DMXAIClient(configuration: config), agnesKeyProvider: { XCTFail("No Agnes fallback"); return nil }, initialProvider: .deepseek, deepSeekKeyProvider: { reads += 1; return "fixture-deepseek" })
        defer { c.close() }
        XCTAssertFalse(c.startGeneration()); XCTAssertEqual(reads, 0)
        c.consentButton.state = .on; XCTAssertTrue(c.startGeneration())
        for _ in 0..<200 where c.isRunning { try await Task.sleep(nanoseconds: 2_000_000) }
        XCTAssertFalse(c.isRunning); XCTAssertEqual(reads, 1)
        XCTAssertEqual(AgnesCopilotProtocol.hosts, ["api.deepseek.com"])
        XCTAssertEqual(AgnesCopilotProtocol.bodies.first?["model"] as? String, "deepseek-flash")
        XCTAssertTrue(c.answerView.isRichText); XCTAssertTrue(c.answerView.string.contains("Synthetic summary"))
        c.selectProvider(.dmx); XCTAssertEqual(c.consentButton.state, .off); XCTAssertTrue(c.conversation.isEmpty)
    }
    func testMissingDeepSeekNeverFallsBackToOtherKeys() {
        let doc = document()
        let c = PDFCopilotController(documentProvider: { doc }, currentPageProvider: { 0 }, keyProvider: { XCTFail("No DMX fallback"); return "fixture-dmx" }, onNavigate: { _ in }, agnesKeyProvider: { XCTFail("No Agnes fallback"); return "fixture-agnes" }, initialProvider: .deepseek)
        defer { c.close() }; c.consentButton.state = .on
        XCTAssertFalse(c.startGeneration()); XCTAssertTrue(c.statusLabel.stringValue.contains("DeepSeek"))
    }
    func testMissingAgnesDoesNotFallBackToDMX() {
        let c = controller(agnesKey: nil); defer { c.close() }; c.consentButton.state = .on
        XCTAssertFalse(c.startGeneration()); XCTAssertTrue(c.statusLabel.stringValue.contains("Agnes"))
    }
    func testLoggedOutAccountIsNotReportedAsKeychainPermissionFailure() {
        let doc = document()
        let c = PDFCopilotController(documentProvider: { doc }, currentPageProvider: { 0 }, keyProvider: { nil }, onNavigate: { _ in }, agnesKeyProvider: { throw AccountStoreFailure.loginRequired }, initialProvider: .agnes)
        defer { c.close() }; c.consentButton.state = .on
        XCTAssertFalse(c.startGeneration())
        XCTAssertEqual(c.statusLabel.stringValue, "请先登录本机账户。")
        XCTAssertFalse(c.isRunning)
    }
    func testProviderSwitchRevokesConsentAndClearsAnswerAndHistory() {
        let c = controller(); defer { c.close() }; c.answerView.string = "old answer"; c.consentButton.state = .on
        c.selectProvider(.dmx)
        XCTAssertEqual(c.answerView.string, ""); XCTAssertEqual(c.consentButton.state, .off)
        XCTAssertEqual(c.modelField.stringValue, DMXAIClient.defaultTextModel)
        XCTAssertTrue(c.consentButton.title.contains("DMX")); XCTAssertTrue(c.conversation.isEmpty)
    }
    func testNativeRichAnswerCitationsNavigateOnlyBeforeRevisionChanges() async throws {
        let doc = document(); let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [AgnesCopilotProtocol.self]
        var revision = 0; var pages: [Int] = []
        let c = PDFCopilotController(documentProvider: { doc }, currentPageProvider: { 0 }, keyProvider: { nil }, onNavigate: { pages.append($0) }, revisionProvider: { revision }, client: DMXAIClient(configuration: config), agnesKeyProvider: { "test-only" }, initialProvider: .agnes)
        defer { c.close() }; c.consentButton.state = .on
        XCTAssertTrue(c.startGeneration())
        for _ in 0..<200 where c.isRunning { try await Task.sleep(nanoseconds: 2_000_000) }
        XCTAssertTrue(c.answerView.isRichText)
        let text = try XCTUnwrap(c.answerView.textStorage)
        let range = (text.string as NSString).range(of: "[p.1]")
        XCTAssertNotEqual(range.location, NSNotFound)
        let link = try XCTUnwrap(text.attribute(.link, at: range.location, effectiveRange: nil) as? URL)
        XCTAssertTrue(c.textView(c.answerView, clickedOnLink: link, at: range.location)); XCTAssertEqual(pages, [0])
        revision += 1
        XCTAssertTrue(c.textView(c.answerView, clickedOnLink: link, at: range.location)); XCTAssertEqual(pages, [0])
        XCTAssertTrue(c.textView(c.answerView, clickedOnLink: URL(string: "https://example.invalid")!, at: 0)); XCTAssertEqual(pages, [0])
    }
    func testFollowupRetainsOnlySameScopeConversationAndNewChatClearsIt() async throws {
        AgnesCopilotProtocol.bodies = []; AgnesCopilotProtocol.hosts = []
        let c = controller(); defer { c.close() }; c.actionChoice.selectItem(at: PDFCopilotAction.allCases.firstIndex(of: .ask)!)
        c.questionField.stringValue = "What is the date?"; c.consentButton.state = .on
        XCTAssertTrue(c.startGeneration())
        for _ in 0..<200 where c.isRunning { try await Task.sleep(nanoseconds: 2_000_000) }
        XCTAssertEqual(c.conversation.count, 2)
        c.questionField.stringValue = "Repeat it in Chinese"; XCTAssertTrue(c.startGeneration())
        for _ in 0..<200 where c.isRunning { try await Task.sleep(nanoseconds: 2_000_000) }
        XCTAssertEqual(AgnesCopilotProtocol.hosts, ["apihub.agnes-ai.com", "apihub.agnes-ai.com"])
        let messages = try XCTUnwrap(AgnesCopilotProtocol.bodies.last?["messages"] as? [[String: Any]])
        let content = try XCTUnwrap(messages.last?["content"] as? String)
        XCTAssertTrue(content.contains("What is the date?")); XCTAssertTrue(content.contains("Synthetic summary"))
        XCTAssertTrue(c.answerView.string.contains("Repeat it in Chinese"))
        c.beginNewChat(); XCTAssertTrue(c.conversation.isEmpty); XCTAssertEqual(c.answerView.string, ""); XCTAssertEqual(c.consentButton.state, .off)
    }
}
