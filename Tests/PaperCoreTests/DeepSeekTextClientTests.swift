import Foundation
import XCTest
@testable import PaperCore

private final class DeepSeekTextProtocol: URLProtocol {
    static var handler: ((URLRequest) -> (Int, Data))?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let (status, data) = Self.handler!(request)
        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data); client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

final class DeepSeekTextClientTests: XCTestCase {
    override func tearDown() { DeepSeekTextProtocol.handler = nil; super.tearDown() }
    private func client() -> DMXAIClient {
        let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [DeepSeekTextProtocol.self]
        return DMXAIClient(configuration: config)
    }
    func testDeepSeekUsesOfficialHostOwnBearerAndNonThinkingFlash() async throws {
        let provider = try XCTUnwrap(PDFTextProvider(rawValue: "deepseek"))
        DeepSeekTextProtocol.handler = { request in
            XCTAssertEqual(request.url?.absoluteString, "https://api.deepseek.com/chat/completions")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer deepseek-test-only")
            var data = request.httpBody ?? Data()
            if let s = request.httpBodyStream { s.open(); defer { s.close() }; var b = [UInt8](repeating: 0, count: 4096); while s.hasBytesAvailable { let n = s.read(&b, maxLength: b.count); if n <= 0 { break }; data.append(contentsOf: b.prefix(n)) } }
            let body = try! JSONSerialization.jsonObject(with: data) as! [String: Any]
            XCTAssertEqual(body["model"] as? String, "deepseek-flash")
            XCTAssertEqual(body["stream"] as? Bool, false)
            XCTAssertEqual((body["thinking"] as? [String: String])?["type"], "disabled")
            return (200, Data(#"{"id":"test","object":"chat.completion","created":0,"model":"deepseek-flash","choices":[{"index":0,"message":{"role":"assistant","content":"**合成摘要** [p.1]"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#.utf8))
        }
        let answer = try await client().completeText(provider: provider, messages: [.init(role: "user", content: "Public synthetic fixture")], key: "deepseek-test-only")
        XCTAssertEqual(answer, "**合成摘要** [p.1]")
    }
    func testDeepSeekNeverRetriesRedirectsOrExposesPrivateResponse() async throws {
        let provider = try XCTUnwrap(PDFTextProvider(rawValue: "deepseek")); var count = 0
        DeepSeekTextProtocol.handler = { _ in count += 1; return (401, Data("private provider details".utf8)) }
        do { _ = try await client().completeText(provider: provider, messages: [.init(role: "user", content: "Test")], key: "test-only"); XCTFail() }
        catch { XCTAssertFalse(error.localizedDescription.contains("private provider")) }
        XCTAssertEqual(count, 1)
        DeepSeekTextProtocol.handler = { _ in count += 1; return (302, Data()) }
        do { _ = try await client().completeText(provider: provider, messages: [.init(role: "user", content: "Test")], key: "test-only"); XCTFail() } catch DMXAIError.redirect {} catch { XCTFail() }
        do { _ = try await client().completeText(provider: provider, messages: [.init(role: "user", content: "Test")], key: "bad\nkey"); XCTFail() } catch DMXAIError.invalidKey {} catch { XCTFail() }
        XCTAssertEqual(count, 2)
    }
}
