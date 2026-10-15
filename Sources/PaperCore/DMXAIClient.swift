import Foundation
import ImageIO

public struct DMXMessage: Codable, Sendable {
    public let role: String
    public let content: String
    public init(role: String, content: String) { self.role = role; self.content = content }
}
public enum DMXVideoResult: Equatable, Sendable { case running, completed(URL), failed }
public enum DMXAIError: Error, LocalizedError {
    case invalidInput, invalidKey, network, http(Int), invalidResponse, tooLarge, redirect, cancelled
    public var errorDescription: String? {
        switch self {
        case .invalidInput: return "请检查提示词、模型名称或任务 ID。"
        case .invalidKey: return "请先在本机账户设置中保存有效的 DMX Key。"
        case .network: return "网络请求未完成；未自动重试。若已提交视频，请使用保存的任务 ID 查询。"
        case .http(let status): return "DMX 请求失败（HTTP \(status)）；未自动重试。请检查账户额度与模型权限。"
        case .invalidResponse: return "服务返回了无法验证的内容。"
        case .tooLarge: return "服务返回内容超过安全大小上限。"
        case .redirect: return "请求被重定向，已停止以保护凭据。"
        case .cancelled: return "已停止本机请求；远端已提交的任务仍可能计费。"
        }
    }
}
public final class DMXAIClient: @unchecked Sendable {
    public static let defaultTextModel = "doubao-seed-2-1-turbo-260628"
    private let configuration: URLSessionConfiguration
    public init(configuration: URLSessionConfiguration = .ephemeral) {
        self.configuration = configuration.copy() as! URLSessionConfiguration
        self.configuration.httpCookieStorage = nil; self.configuration.urlCache = nil
        self.configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        self.configuration.httpAdditionalHeaders = [:]
    }
    public func generateImage(prompt: String, key: String) async throws -> Data {
        try validatePrompt(prompt)
        let json = try await post("images/generations", body: ["model": "gpt-image-2.5-sunburst", "prompt": prompt, "n": 1, "size": "1024x1024"], key: key, timeout: 300, limit: 68 * 1024 * 1024)
        guard let items = json["data"] as? [[String: Any]], var encoded = items.first?["b64_json"] as? String else { throw DMXAIError.invalidResponse }
        if encoded.hasPrefix("data:image/"), let comma = encoded.firstIndex(of: ",") { encoded = String(encoded[encoded.index(after: comma)...]) }
        guard let bytes = Data(base64Encoded: encoded), bytes.count <= 48 * 1024 * 1024,
              let source = CGImageSourceCreateWithData(bytes as CFData, nil), CGImageSourceGetCount(source) == 1,
              let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = props[kCGImagePropertyPixelWidth] as? Int, let height = props[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, width <= 8192, height <= 8192, width * height <= 16_777_216,
              CGImageSourceCreateImageAtIndex(source, 0, nil) != nil else { throw DMXAIError.invalidResponse }
        return bytes
    }
    public func submitVideo(prompt: String, key: String) async throws -> String {
        try validatePrompt(prompt)
        let json = try await post("responses", body: ["model": "doubao-seedance-2-5-260628", "input": [["type": "text", "text": prompt]], "ratio": "16:9", "resolution": "480p", "duration": 4, "generate_audio": false, "watermark": true], key: key, timeout: 120, limit: 1024 * 1024)
        guard let id = json["id"] as? String, Self.validTaskID(id) else { throw DMXAIError.invalidResponse }
        return id
    }
    public func queryVideo(taskID: String, key: String) async throws -> DMXVideoResult {
        guard Self.validTaskID(taskID) else { throw DMXAIError.invalidInput }
        let json = try await post("responses", body: ["model": "seedance-2-5-get", "input": taskID], key: key, timeout: 60, limit: 1024 * 1024)
        let payload = json["payload"] as? [String: Any] ?? json
        var state = (payload["status"] as? String ?? json["status"] as? String ?? "").lowercased()
        if ["failed", "cancelled", "canceled", "error"].contains(state) || payload["error"] != nil { return .failed }
        let output = payload["output"] as? [[String: Any]]
        let content = output?.first?["content"] as? [[String: Any]]
        let text = (content?.first?["text"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        var candidate: String?
        if text.hasPrefix("https://") { candidate = text }
        else if !text.isEmpty {
            guard let decoded = try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any] else { throw DMXAIError.invalidResponse }
            let task = decoded["payload"] as? [String: Any] ?? decoded
            let detail = task["content"] as? [String: Any]
            state = (task["status"] as? String ?? detail?["status"] as? String ?? state).lowercased()
            if task["error"] != nil || detail?["error"] != nil || ["failed", "cancelled", "canceled", "error"].contains(state) { return .failed }
            candidate = detail?["video_url"] as? String ?? task["video_url"] as? String
        }
        if let candidate {
            guard let url = URL(string: candidate), Self.validMediaURL(url) else { throw DMXAIError.invalidResponse }
            return .completed(url)
        }
        return ["succeeded", "completed", "success"].contains(state) ? .failed : .running
    }
    public func completeText(model: String = defaultTextModel, messages: [DMXMessage], key: String) async throws -> String {
        guard !model.isEmpty, model.count <= 160, model.range(of: "^[A-Za-z0-9._:/-]+$", options: .regularExpression) != nil,
              !messages.isEmpty, messages.count <= 16, messages.allSatisfy({ ["system", "user", "assistant"].contains($0.role) }),
              messages.reduce(0, { $0 + $1.content.count }) <= 48_000 else { throw DMXAIError.invalidInput }
        let json = try await post("chat/completions", body: ["model": model, "messages": messages.map { ["role": $0.role, "content": $0.content] }, "stream": false, "max_tokens": 4096], key: key, timeout: 120, limit: 1024 * 1024)
        guard let choices = json["choices"] as? [[String: Any]], let message = choices.first?["message"] as? [String: Any],
              let text = message["content"] as? String, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, text.count <= 100_000 else { throw DMXAIError.invalidResponse }
        return text
    }
    public func downloadVideo(url: URL) async throws -> Data {
        guard Self.validMediaURL(url) else { throw DMXAIError.invalidInput }
        var request = URLRequest(url: url); request.httpMethod = "GET"; request.timeoutInterval = 300
        request.setValue("video/mp4", forHTTPHeaderField: "Accept")
        let bytes = try await DMXBoundedTransport(configuration: configuration, limit: 96 * 1024 * 1024).run(request)
        guard bytes.count >= 12, bytes.subdata(in: 4..<8) == Data("ftyp".utf8) else { throw DMXAIError.invalidResponse }
        return bytes
    }
    public static func validTaskID(_ id: String) -> Bool {
        !id.isEmpty && id.count <= 256 && id.range(of: "^[A-Za-z0-9_-]+$", options: .regularExpression) != nil
    }
    private static func validMediaURL(_ url: URL) -> Bool {
        guard url.scheme == "https", let host = url.host?.lowercased(), host.contains("."), url.user == nil, url.password == nil, url.fragment == nil,
              url.port == nil || url.port == 443, !host.hasSuffix(".local"), !host.hasSuffix(".localhost"),
              host.range(of: "^[0-9.]+$", options: .regularExpression) == nil, !host.contains(":") else { return false }
        return true
    }
    private func validatePrompt(_ prompt: String) throws {
        guard !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, prompt.count <= 32_000 else { throw DMXAIError.invalidInput }
    }
    private func post(_ path: String, body: [String: Any], key: String, timeout: TimeInterval, limit: Int) async throws -> [String: Any] {
        guard !key.isEmpty, key.count <= 1024, !key.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) || CharacterSet.whitespacesAndNewlines.contains($0) }) else { throw DMXAIError.invalidKey }
        var request = URLRequest(url: URL(string: "https://www.dmxapi.cn/v1/" + path)!)
        request.httpMethod = "POST"; request.timeoutInterval = timeout
        request.setValue("Bearer " + key, forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type"); request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        let data = try await DMXBoundedTransport(configuration: configuration, limit: limit).run(request)
        guard let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any], json["error"] == nil else { throw DMXAIError.invalidResponse }
        return json
    }
}

/// One explicit request. No retry, no redirect, no logs, bounded before and during download.
private final class DMXBoundedTransport: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    private let configuration: URLSessionConfiguration
    private let limit: Int
    private let lock = NSLock()
    private var task: URLSessionDataTask?
    private var continuation: CheckedContinuation<Data, Error>?
    private var bytes = Data()
    private var failure: DMXAIError?
    private var wasCancelled = false
    init(configuration: URLSessionConfiguration, limit: Int) { self.configuration = configuration; self.limit = limit }
    func run(_ request: URLRequest) async throws -> Data {
        if Task.isCancelled { throw DMXAIError.cancelled }
        return try await withTaskCancellationHandler(operation: {
            try await withCheckedThrowingContinuation { continuation in
                self.lock.lock()
                if self.wasCancelled { self.lock.unlock(); continuation.resume(throwing: DMXAIError.cancelled); return }
                self.continuation = continuation
                let config = self.configuration.copy() as! URLSessionConfiguration
                config.timeoutIntervalForResource = request.timeoutInterval
                let session = URLSession(configuration: config, delegate: self, delegateQueue: nil)
                let task = session.dataTask(with: request); self.task = task
                self.lock.unlock(); task.resume()
            }
        }, onCancel: { self.lock.lock(); self.wasCancelled = true; self.failure = .cancelled; let task = self.task; self.lock.unlock(); task?.cancel() })
    }
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        lock.lock(); failure = .redirect; lock.unlock(); completionHandler(nil)
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse, completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        lock.lock()
        if let http = response as? HTTPURLResponse {
            if (300..<400).contains(http.statusCode) { failure = .redirect }
            else if !(200..<300).contains(http.statusCode) { failure = .http(http.statusCode) }
        } else { failure = .invalidResponse }
        if response.expectedContentLength > Int64(limit) { failure = .tooLarge }
        let reject = failure != nil; lock.unlock(); completionHandler(reject ? .cancel : .allow)
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        lock.lock()
        if data.count > limit - bytes.count { failure = .tooLarge; lock.unlock(); dataTask.cancel(); return }
        bytes.append(data); lock.unlock()
    }
    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        lock.lock(); let continuation = self.continuation; self.continuation = nil
        let result: Result<Data, Error>
        if let failure { result = .failure(failure) }
        else if error != nil { result = .failure(DMXAIError.network) }
        else { result = .success(bytes) }
        self.task = nil; bytes = Data(); lock.unlock()
        continuation?.resume(with: result); session.finishTasksAndInvalidate()
    }
}
