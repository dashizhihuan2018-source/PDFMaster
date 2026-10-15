import AppKit
import XCTest
@testable import PaperCore

private final class DMXMockProtocol: URLProtocol {
    static var handler: ((URLRequest) throws -> (Int, Data))?
    static var headers: [String: String] = [:]
    static var didStart: (() -> Void)?
    static var remainPending = false
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        Self.didStart?()
        if Self.remainPending { return }
        do {
            let (status, data) = try Self.handler!(request)
            let response = HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: ["Content-Type": "application/json"].merging(Self.headers) { _, value in value })!
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}
}

final class DMXAIClientTests: XCTestCase {
    private func makeClient() -> DMXAIClient {
        let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [DMXMockProtocol.self]
        return DMXAIClient(configuration: config)
    }
    override func tearDown() { DMXMockProtocol.handler = nil; DMXMockProtocol.headers = [:]; DMXMockProtocol.didStart = nil; DMXMockProtocol.remainPending = false; super.tearDown() }
    private func body(_ request: URLRequest) throws -> [String: Any] {
        if let data = request.httpBody { return try JSONSerialization.jsonObject(with: data) as! [String: Any] }
        let stream = try XCTUnwrap(request.httpBodyStream); stream.open(); defer { stream.close() }
        var data = Data(); var buffer = [UInt8](repeating: 0, count: 4096)
        while stream.hasBytesAvailable { let n = stream.read(&buffer, maxLength: buffer.count); if n <= 0 { break }; data.append(contentsOf: buffer.prefix(n)) }
        return try JSONSerialization.jsonObject(with: data) as! [String: Any]
    }
    func testImageRequestUsesFixedDMXModelAndReturnsDecodedPNG() async throws {
        let image = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        let png = image.representation(using: .png, properties: [:])!
        DMXMockProtocol.handler = { request in
            XCTAssertEqual(request.url?.absoluteString, "https://www.dmxapi.cn/v1/images/generations")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer unit-test-key")
            let json = try self.body(request)
            XCTAssertEqual(json["model"] as? String, "gpt-image-2.5-sunburst")
            XCTAssertEqual(json["n"] as? Int, 1); XCTAssertEqual(json["size"] as? String, "1024x1024")
            return (200, try JSONSerialization.data(withJSONObject: ["data": [["b64_json": png.base64EncodedString()]]]))
        }
        let result = try await makeClient().generateImage(prompt: "test", key: "unit-test-key")
        XCTAssertEqual(result, png)
    }
    func testVideoSubmitAcceptsOfficialTaskIDAndKeepsWatermark() async throws {
        DMXMockProtocol.handler = { request in
            let json = try self.body(request)
            XCTAssertEqual(json["model"] as? String, "doubao-seedance-2-5-260628")
            XCTAssertEqual(json["duration"] as? Int, 4); XCTAssertEqual(json["watermark"] as? Bool, true)
            XCTAssertEqual(json["generate_audio"] as? Bool, false)
            return (200, Data(#"{"id":"task_8AAJeEO9Gf0GiSh0"}"#.utf8))
        }
        let id = try await makeClient().submitVideo(prompt: "test", key: "unit-test-key")
        XCTAssertEqual(id, "task_8AAJeEO9Gf0GiSh0")
    }
    func testQueryParsesNestedJSONVideoResult() async throws {
        DMXMockProtocol.handler = { request in
            let json = try self.body(request); XCTAssertEqual(json["model"] as? String, "seedance-2-5-get")
            return (200, Data(#"{"payload":{"output":[{"content":[{"text":"{\"status\":\"succeeded\",\"content\":{\"video_url\":\"https://media.example.com/result.mp4?signature=private\"}}"}]}]}}"#.utf8))
        }
        let result = try await makeClient().queryVideo(taskID: "cgt-abcdefgh", key: "unit-test-key")
        guard case .completed(let url) = result else { return XCTFail("Expected completed") }
        XCTAssertEqual(url.host, "media.example.com")
    }
    func testQueryRunningDoesNotResubmit() async throws {
        var requests = 0
        DMXMockProtocol.handler = { _ in requests += 1; return (200, Data(#"{"status":"running"}"#.utf8)) }
        let result = try await makeClient().queryVideo(taskID: "task_abcdefgh", key: "unit-test-key")
        XCTAssertEqual(result, .running); XCTAssertEqual(requests, 1)
    }
    func testTextUsesChatCompletionAndParsesContent() async throws {
        DMXMockProtocol.handler = { request in
            XCTAssertEqual(request.url?.path, "/v1/chat/completions")
            let json = try self.body(request); XCTAssertEqual(json["stream"] as? Bool, false)
            return (200, Data(#"{"choices":[{"message":{"content":"summary [p.1]"}}]}"#.utf8))
        }
        let text = try await makeClient().completeText(model: DMXAIClient.defaultTextModel, messages: [.init(role: "user", content: "test")], key: "unit-test-key")
        XCTAssertEqual(text, "summary [p.1]")
    }
    func testProviderFailureIsGenericAndDoesNotRetry() async {
        var requests = 0
        DMXMockProtocol.handler = { _ in requests += 1; return (401, Data("unit-test-key https://media.example.com/?secret=yes".utf8)) }
        do { _ = try await makeClient().submitVideo(prompt: "test", key: "unit-test-key"); XCTFail() }
        catch { XCTAssertFalse(error.localizedDescription.contains("unit-test-key")); XCTAssertFalse(error.localizedDescription.contains("secret")) }
        XCTAssertEqual(requests, 1)
    }
    func testInvalidImageAndRedirectResponseAreRejected() async {
        for (status, data) in [(200, Data(#"{"data":[{"b64_json":"YWJjZA=="}]}"#.utf8)), (302, Data())] {
            DMXMockProtocol.handler = { _ in (status, data) }
            do { _ = try await makeClient().generateImage(prompt: "test", key: "unit-test-key"); XCTFail() } catch {}
        }
    }
    func testCancellationBeforeRequestAndInvalidKeySendNothing() async {
        var requests = 0; DMXMockProtocol.handler = { _ in requests += 1; return (200, Data()) }
        do { _ = try await makeClient().submitVideo(prompt: "test", key: "bad\nkey"); XCTFail() } catch {}
        let task = Task { try await self.makeClient().submitVideo(prompt: "test", key: "unit-test-key") }
        task.cancel(); _ = await task.result
        XCTAssertEqual(requests, 0)
    }
    func testVideoDownloadNeverSendsAuthorization() async throws {
        let mp4 = Data([0, 0, 0, 24]) + Data("ftypisom".utf8)
        DMXMockProtocol.handler = { request in
            XCTAssertNil(request.value(forHTTPHeaderField: "Authorization")); XCTAssertEqual(request.httpMethod, "GET")
            return (200, mp4)
        }
        let data = try await makeClient().downloadVideo(url: URL(string: "https://media.example.com/test.mp4")!)
        XCTAssertEqual(data, mp4)
    }
    func testResponseDeclaredAboveCapIsStoppedWithoutParsing() async {
        DMXMockProtocol.headers = ["Content-Length": "1048577"]
        DMXMockProtocol.handler = { _ in (200, Data(#"{"id":"task_allowed"}"#.utf8)) }
        do { _ = try await makeClient().submitVideo(prompt: "test", key: "unit-test-key"); XCTFail() }
        catch DMXAIError.tooLarge {} catch { XCTFail("Expected bounded response rejection") }
    }
    func testInvalidTaskURLAndProviderTaskFailureNeverDownload() async throws {
        DMXMockProtocol.handler = { _ in (200, Data(#"{"status":"failed"}"#.utf8)) }
        let failed = try await makeClient().queryVideo(taskID: "task_failed", key: "unit-test-key")
        XCTAssertEqual(failed, .failed)
        DMXMockProtocol.handler = { _ in (200, Data(#"{"output":[{"content":[{"text":"https://127.0.0.1/private.mp4"}]}]}"#.utf8)) }
        do { _ = try await makeClient().queryVideo(taskID: "task_private", key: "unit-test-key"); XCTFail() } catch {}
    }
    func testCancellationOfInFlightRequestCompletesWithoutLeakingNetworkError() async {
        let started = expectation(description: "Request started"); DMXMockProtocol.remainPending = true
        DMXMockProtocol.didStart = { started.fulfill() }
        let client = makeClient(); let request = Task { try await client.submitVideo(prompt: "test", key: "unit-test-key") }
        await fulfillment(of: [started], timeout: 2); request.cancel()
        do { _ = try await request.value; XCTFail() } catch DMXAIError.cancelled {} catch { XCTFail("Expected cancellation") }
    }
    func testMalformedTextAndInvalidMP4AreRejected() async {
        DMXMockProtocol.handler = { _ in (200, Data(#"{"choices":[{"message":{"content":null}}]}"#.utf8)) }
        do { _ = try await makeClient().completeText(messages: [.init(role: "user", content: "test")], key: "unit-test-key"); XCTFail() } catch {}
        do { _ = try await makeClient().downloadVideo(url: URL(string: "https://media.example.com/bad.mp4")!); XCTFail() } catch {}
    }
}
