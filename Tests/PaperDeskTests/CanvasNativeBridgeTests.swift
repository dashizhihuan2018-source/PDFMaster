import AppKit
import PaperCore
import WebKit
import XCTest
@testable import PaperDesk

@MainActor private final class CanvasTestSecrets: AccountSecretStorage {
    var data: [String: String] = [:]
    func read(service: String, account: String) throws -> String? { data[service + account] }
    func save(_ value: String, service: String, account: String) throws { data[service + account] = value }
    func delete(service: String, account: String) throws { data.removeValue(forKey: service + account) }
}
private final class CanvasMockProtocol: URLProtocol {
    static var handler: ((URLRequest) throws -> (Int, Data))?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        do {
            let (status, bytes) = try Self.handler!(request)
            client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: bytes); client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}
}
@MainActor final class CanvasNativeBridgeTests: XCTestCase {
    private func bridge() -> CanvasNativeBridge {
        let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [CanvasMockProtocol.self]
        return CanvasNativeBridge(secrets: CanvasTestSecrets(), namespace: "test", client: DMXAIClient(configuration: config))
    }
    private func request(_ path: String = "/api/dmx/v1/images/generations") -> [String: Any] {
        ["operation": "request", "path": path, "method": "POST", "contentType": "application/json", "body": Data(#"{"model":"gpt-image-2.5-sunburst","prompt":"fixture","n":1}"#.utf8).base64EncodedString()]
    }
    func testKeyCanBeSavedWithoutAccountAndNeverReturnedToWeb() async throws {
        let bridge = bridge(); try bridge.saveKey("  fixture-key  ")
        XCTAssertEqual(try bridge.readKey(), "fixture-key")
        for value in ["", "a b", "a\nb"] { XCTAssertThrowsError(try bridge.saveKey(value)) }
        let response = await bridge.handle(["operation": "request", "path": "/api/auth/verify", "method": "GET"])
        XCTAssertFalse(String(describing: response).contains("fixture-key"))
        try bridge.removeKey(); XCTAssertNil(try bridge.readKey())
    }
    func testLocalExportPreservesFilenameAndRequiresNativeSaveChoice() async throws {
        let bridge = bridge()
        var received = false
        bridge.onExport = { data, name in
            XCTAssertEqual(data, Data("zip-fixture".utf8))
            XCTAssertEqual(name, "我的画布.zip")
            received = true
            return false // User cancelled: no file is written.
        }
        let result = await bridge.handle(["operation": "export", "name": "我的画布.zip", "body": Data("zip-fixture".utf8).base64EncodedString()])
        XCTAssertTrue(received)
        XCTAssertEqual(result["saved"] as? Bool, false)
        for name in ["../private.zip", "/tmp/private.zip", "", "a\\b.zip"] {
            received = false
            _ = await bridge.handle(["operation": "export", "name": name, "body": ""])
            XCTAssertFalse(received)
        }
    }
    func testMissingKeyOrDeclinedConsentNeverCallsProvider() async throws {
        let bridge = bridge(); var requests = 0; var confirmations = 0
        CanvasMockProtocol.handler = { _ in requests += 1; return (200, Data()) }
        defer { CanvasMockProtocol.handler = nil }
        bridge.confirmSend = { _ in confirmations += 1; return false }
        let missing = await bridge.handle(request()); XCTAssertEqual(missing["status"] as? Int, 400)
        XCTAssertEqual(confirmations, 0)
        try bridge.saveKey("fixture-key")
        let declined = await bridge.handle(request()); XCTAssertEqual(declined["status"] as? Int, 400)
        XCTAssertEqual(confirmations, 1); XCTAssertEqual(requests, 0)
    }
    func testOnlyFixedDMXEndpointReceivesNativeKeyAndNoRetry() async throws {
        let bridge = bridge(); try bridge.saveKey("fixture-key"); bridge.confirmSend = { _ in true }
        var count = 0
        CanvasMockProtocol.handler = { request in
            count += 1
            XCTAssertEqual(request.url?.absoluteString, "https://www.dmxapi.cn/v1/images/generations")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer fixture-key")
            return (401, Data("fixture-key upstream-private-data".utf8))
        }
        defer { CanvasMockProtocol.handler = nil }
        let failed = await bridge.handle(request())
        XCTAssertEqual(count, 1); XCTAssertEqual(failed["status"] as? Int, 400)
        let text = String(data: Data(base64Encoded: failed["body"] as! String)!, encoding: .utf8)!
        XCTAssertFalse(text.contains("fixture-key")); XCTAssertFalse(text.contains("upstream-private"))
        for path in ["https://evil.example/v1/images/generations", "/api/dmx/v1/../responses", "/api/agnes/v1/responses", "/api/dmx/v1/models"] {
            let rejected = await bridge.handle(request(path)); XCTAssertEqual(rejected["status"] as? Int, 400)
        }
        XCTAssertEqual(count, 1)
    }
    func testAssetRootRejectsTraversalAndNeverFallsBackToWebsite() {
        let scheme = CanvasBundleScheme()
        XCTAssertNotNil(scheme.asset(for: URL(string: "pdfmaster-canvas://workspace/canvas")!))
        for address in ["file:///etc/passwd", "pdfmaster-canvas://workspace/%2e%2e/Info.plist", "pdfmaster-canvas://workspace/api/private", "https://infinitecanvas.maip.top/"] {
            XCTAssertNil(scheme.asset(for: URL(string: address)!), address)
        }
    }
    func testCandidateDataAndKeyNamespacesAreSeparateFromInstalledApp() throws {
        let normal = "cn.nankong.pdfmaster.desktop", candidate = "cn.nankong.pdfmaster.validation.canvas28"
        XCTAssertNotEqual(CanvasWebsiteController.dataStoreID(normal), CanvasWebsiteController.dataStoreID(candidate))
        XCTAssertEqual(CanvasWebsiteController.dataStoreID(candidate), CanvasWebsiteController.dataStoreID(candidate))
        let secrets = CanvasTestSecrets()
        let original = CanvasNativeBridge(secrets: secrets, namespace: normal)
        let preview = CanvasNativeBridge(secrets: secrets, namespace: candidate)
        try original.saveKey("original-fixture-key")
        XCTAssertNil(try preview.readKey())
        try preview.saveKey("preview-fixture-key"); try preview.removeKey()
        XCTAssertEqual(try original.readKey(), "original-fixture-key")
    }
    func testVideoTaskQueriesNeverResubmitAndMediaDownloadHasNoKey() async throws {
        let bridge = bridge(); try bridge.saveKey("fixture-key"); bridge.confirmSend = { _ in true }
        var paths: [String] = []
        CanvasMockProtocol.handler = { request in
            paths.append(request.url!.path)
            if request.url?.host == "media.example.com" {
                XCTAssertNil(request.value(forHTTPHeaderField: "Authorization"))
                return (200, Data([0, 0, 0, 20] + Array("ftypisom".utf8)))
            }
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer fixture-key")
            if paths.count == 1 { return (200, Data(#"{"id":"task-fixture"}"#.utf8)) }
            return (200, Data(#"{"payload":{"output":[{"content":[{"text":"https://media.example.com/result.mp4"}]}]}}"#.utf8))
        }
        defer { CanvasMockProtocol.handler = nil }
        let submitted = await bridge.handle(["operation": "video-submit", "model": "doubao-seedance-2-5-260628", "prompt": "fixture", "duration": 4])
        XCTAssertEqual(submitted["id"] as? String, "task-fixture")
        let status = await bridge.handle(["operation": "request", "method": "GET", "path": "/api/dmx/v1/videos/task-fixture"])
        XCTAssertEqual(status["status"] as? Int, 200)
        let downloaded = await bridge.handle(["operation": "request", "method": "GET", "path": "/api/dmx/v1/videos/task-fixture/content"])
        XCTAssertEqual(downloaded["contentType"] as? String, "video/mp4")
        XCTAssertEqual(paths, ["/v1/responses", "/v1/responses", "/result.mp4"])
    }
    func testBundledAppRendersOfflineAndMissingKeyErrorReachesUIBridge() async throws {
        let controller = CanvasWebsiteController(bridge: bridge()); _ = controller.view
        controller.view.frame = CGRect(x: 0, y: 0, width: 1200, height: 800)
        let web = controller.webView!
        var text = ""
        for _ in 0..<100 {
            text = (try? await web.evaluateJavaScript("document.body.innerText")) as? String ?? ""
            if text.contains("DMX Key") { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        XCTAssertTrue(text.contains("DMX Key"), text)
        XCTAssertFalse(text.contains("Application Error"), text)
        let response = try await web.callAsyncJavaScript("const response = await fetch('/api/dmx/v1/images/generations',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({prompt:'fixture'})}); return {status:response.status,body:await response.text()};", arguments: [:], in: nil, contentWorld: .page) as? [String: Any]
        XCTAssertEqual(response?["status"] as? Int, 400)
        XCTAssertTrue((response?["body"] as? String)?.contains("Key") == true)
        web.stopLoading()
    }
}
