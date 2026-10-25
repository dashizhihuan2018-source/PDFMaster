import Foundation
import XCTest
@testable import PaperCore

private final class AgnesTextProtocol: URLProtocol {
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

final class AgnesTextClientTests: XCTestCase {
    private func client() -> DMXAIClient {
        let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [AgnesTextProtocol.self]
        return DMXAIClient(configuration: config)
    }
    override func tearDown() { AgnesTextProtocol.handler = nil; super.tearDown() }
    func testAgnesTextUsesOwnFixedHostAndBearerWithoutStreaming() async throws {
        AgnesTextProtocol.handler = { request in
            XCTAssertEqual(request.url?.absoluteString, "https://apihub.agnes-ai.com/v1/chat/completions")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer agnes-unit-test")
            var data = request.httpBody ?? Data()
            if let stream = request.httpBodyStream { stream.open(); defer { stream.close() }; var buffer = [UInt8](repeating: 0, count: 4096); while stream.hasBytesAvailable { let n = stream.read(&buffer, maxLength: buffer.count); if n <= 0 { break }; data.append(contentsOf: buffer.prefix(n)) } }
            let body = try! JSONSerialization.jsonObject(with: data) as! [String: Any]
            XCTAssertEqual(body["model"] as? String, PDFTextProvider.agnes.defaultModel)
            XCTAssertEqual(body["stream"] as? Bool, false)
            return (200, Data(#"{"choices":[{"message":{"content":"合成文档摘要 [p.1]"}}]}"#.utf8))
        }
        let answer = try await client().completeText(provider: .agnes, messages: [.init(role: "user", content: "Synthetic fixture")], key: "agnes-unit-test")
        XCTAssertEqual(answer, "合成文档摘要 [p.1]")
    }
    func testAgnesNeverRetriesOrLeaksProviderResponse() async {
        var count = 0
        AgnesTextProtocol.handler = { _ in count += 1; return (401, Data("agnes-unit-test private upstream details".utf8)) }
        do { _ = try await client().completeText(provider: .agnes, messages: [.init(role: "user", content: "Test")], key: "agnes-unit-test"); XCTFail() }
        catch { XCTAssertFalse(error.localizedDescription.contains("agnes-unit-test")); XCTAssertFalse(error.localizedDescription.contains("private")) }
        XCTAssertEqual(count, 1)
    }
    func testAgnesRedirectIsRejectedAndInvalidKeyNeverSends() async {
        var count = 0
        AgnesTextProtocol.handler = { _ in count += 1; return (302, Data()) }
        do { _ = try await client().completeText(provider: .agnes, messages: [.init(role: "user", content: "Test")], key: "test"); XCTFail() } catch DMXAIError.redirect {} catch { XCTFail() }
        do { _ = try await client().completeText(provider: .agnes, messages: [.init(role: "user", content: "Test")], key: "bad\nkey"); XCTFail() } catch DMXAIError.invalidKey {} catch { XCTFail() }
        XCTAssertEqual(count, 1)
    }
}
