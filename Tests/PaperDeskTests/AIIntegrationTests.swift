import AppKit
import PaperCore
import XCTest
@testable import PaperDesk

private final class SummaryAIProtocol: URLProtocol {
    static var handler: ((SummaryAIProtocol) -> Void)?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() { Self.handler?(self) }
    override func stopLoading() {}
    func finish(_ json: String) {
        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(json.utf8)); client?.urlProtocolDidFinishLoading(self)
    }
}

@MainActor final class AIIntegrationTests: XCTestCase {
    func testValidationCredentialsDoNotOverlapInstalledProfile() throws {
        let secrets = MemorySecretStorage()
        try secrets.save("installed-test-value", service: "cn.nankong.pdfmaster.ai", account: "integration")
        let scoped = ProfileAccountSecretStorage(base: secrets, namespace: "validation17")
        XCTAssertNil(try scoped.read(service: "cn.nankong.pdfmaster.ai", account: "integration"))
        try scoped.save("validation-test-value", service: "cn.nankong.pdfmaster.ai", account: "integration")
        XCTAssertEqual(try secrets.read(service: "cn.nankong.pdfmaster.ai", account: "integration"), "installed-test-value")
        try scoped.delete(service: "cn.nankong.pdfmaster.ai", account: "integration")
        XCTAssertEqual(try secrets.read(service: "cn.nankong.pdfmaster.ai", account: "integration"), "installed-test-value")
    }
    private func fixture() throws -> (ApplicationShellController, AccountStore, UserDefaults) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("SummaryTests-" + UUID().uuidString)
        let store = AccountStore(metadataURL: directory.appendingPathComponent("accounts.json"), secretStorage: MemorySecretStorage())
        try store.register(username: "Integration", password: "integration-password")
        try store.login(username: "Integration", password: "integration-password", remember: false)
        let defaults = UserDefaults(suiteName: "SummaryTests-" + UUID().uuidString)!
        let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [SummaryAIProtocol.self]
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: directory.appendingPathComponent("recent.json")), accountStore: store, aiPreferences: defaults, aiClient: DMXAIClient(configuration: config))
        shell.aiStudioPresenter = { _ in }; shell.copilotPresenter = { _ in }; shell.accountSettingsPresenter = { _ in }
        return (shell, store, defaults)
    }
    func testSettingsControllerIsRetainedAndUsesInjectedAccount() throws {
        let (shell, store, _) = try fixture()
        shell.showAccountSettings()
        let first = try XCTUnwrap(shell.accountSettingsController); _ = first.view
        XCTAssertTrue(first.accountStatus.stringValue.contains("Integration"))
        shell.showAccountSettings(); XCTAssertTrue(shell.accountSettingsController === first)
        first.keyField.stringValue = "integration-test-key"; first.performSaveKey()
        XCTAssertEqual(try store.readDMXKey(), "integration-test-key"); XCTAssertEqual(first.keyField.stringValue, "")
    }
    func testCanvasAIMissingKeyDoesNotSendOrInsert() throws {
        let (shell, _, _) = try fixture(); let tab = shell.createCanvas()
        let studio = try XCTUnwrap(shell.showAIStudio(for: tab.controller))
        SummaryAIProtocol.handler = { _ in XCTFail("Missing key must not send") }
        defer { SummaryAIProtocol.handler = nil; studio.close() }
        studio.promptField.string = "A drawing"; studio.consentButton.state = .on
        XCTAssertFalse(studio.startGeneration()); XCTAssertTrue(tab.controller.session.board.elements.isEmpty)
    }
    func testPendingVideoSavesToInitiatingAccountAfterLogout() async throws {
        let (shell, store, defaults) = try fixture(); try store.saveDMXKey("integration-test-key")
        let tab = shell.createCanvas(); let studio = try XCTUnwrap(shell.showAIStudio(for: tab.controller))
        let started = expectation(description: "submitted"); var pending: SummaryAIProtocol?
        SummaryAIProtocol.handler = { pending = $0; started.fulfill() }
        defer { SummaryAIProtocol.handler = nil; studio.close() }
        studio.modeChoice.selectItem(at: 1); studio.promptField.string = "Short film"; studio.consentButton.state = .on
        XCTAssertTrue(studio.startGeneration()); await fulfillment(of: [started], timeout: 2)
        store.logout(); try XCTUnwrap(pending).finish(#"{"id":"task_integration"}"#)
        for _ in 0..<100 where defaults.string(forKey: "PDFMaster.VideoTask.integration") == nil { try await Task.sleep(nanoseconds: 2_000_000) }
        XCTAssertEqual(defaults.string(forKey: "PDFMaster.VideoTask.integration"), "task_integration")
        XCTAssertEqual(studio.taskIDField.stringValue, "")
        XCTAssertNil(defaults.string(forKey: "PDFMaster.VideoTask.guest"))
    }
    func testCompletedCopilotAnswerClearsWhenSwitchingDocument() async throws {
        let (shell, store, _) = try fixture(); try store.saveAgnesKey("integration-test-key")
        let text = NSTextView(frame: CGRect(x: 0, y: 0, width: 300, height: 300)); text.string = "Public integration fixture"
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".pdf")
        try text.dataWithPDF(inside: text.bounds).write(to: url)
        XCTAssertTrue(shell.open(url: url))
        XCTAssertTrue(shell.dispatch(toolID: "copilot", openProvider: { XCTFail("Already open"); return nil }))
        let copilot = try XCTUnwrap(shell.copilotController)
        XCTAssertEqual(copilot.selectedProvider, .agnes)
        SummaryAIProtocol.handler = { $0.finish(#"{"choices":[{"message":{"role":"assistant","content":"Summary [p.1]"}}]}"#) }
        defer { SummaryAIProtocol.handler = nil; copilot.close() }
        copilot.consentButton.state = .on; XCTAssertTrue(copilot.startGeneration())
        for _ in 0..<100 where copilot.isRunning { try await Task.sleep(nanoseconds: 2_000_000) }
        XCTAssertFalse(copilot.answerView.string.isEmpty)
        _ = shell.createBlankDocument()
        XCTAssertEqual(copilot.answerView.string, "", "Old document answer must disappear")
        XCTAssertEqual(copilot.consentButton.state, .off)
    }
    func testCanvasCreationConnectsExplicitAIEntry() {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).appendingPathComponent("recent.json")
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: url))
        let tab = shell.createCanvas()
        XCTAssertNotNil(tab.controller.onShowAI, "Canvas AI must be connected by the shell")
    }
    func testCopilotCatalogIsPartialRatherThanAPlaceholder() {
        XCTAssertEqual(ToolCatalog.descriptor(id: "copilot")?.availability, .partial)
    }
    func testReaderAIIsAnEnabledExplicitEntry() {
        let controller = WorkbenchController(); _ = controller.view
        try? controller.workspace.createBlank(); controller.refreshDocument()
        func buttons(_ view: NSView) -> [NSButton] {
            (view as? NSButton).map { [$0] } ?? view.subviews.flatMap(buttons)
        }
        let ai = buttons(controller.view).first { $0.accessibilityLabel() == "PDF Copilot" }
        XCTAssertNotNil(ai); XCTAssertEqual(ai?.isEnabled, true)
    }
}
