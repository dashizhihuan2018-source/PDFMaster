import AppKit
import PaperCore
import WebKit

/// No local HTTP server or website session. Credential access stays in the native process.
@MainActor final class CanvasNativeBridge: NSObject, WKScriptMessageHandlerWithReply {
    private let secrets: AccountSecretStorage
    private let service: String
    private let client: DMXAIClient
    var onSettings: (() -> Void)?
    var onExport: ((Data, String) async -> Bool)?
    var confirmSend: ((String) async -> Bool)?
    private var media: [String: URL] = [:]
    private var generationEpoch = UUID()

    init(secrets: AccountSecretStorage? = nil, namespace: String = Bundle.main.bundleIdentifier ?? "cn.nankong.pdfmaster.desktop", client: DMXAIClient = DMXAIClient()) {
        self.secrets = secrets ?? KeychainAccountSecretStorage(); self.service = namespace + ".canvas.dmx"; self.client = client
    }
    func readKey() throws -> String? { try secrets.read(service: service, account: "user") }
    func saveKey(_ input: String) throws {
        let key = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty, key.count <= 1024, !key.unicodeScalars.contains(where: { CharacterSet.whitespacesAndNewlines.union(.controlCharacters).contains($0) }) else { throw AccountStoreFailure.invalidKey }
        try secrets.save(key, service: service, account: "user"); revoke()
    }
    func removeKey() throws { try secrets.delete(service: service, account: "user"); revoke() }
    func revoke() { generationEpoch = UUID(); media.removeAll() }

    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage, replyHandler: @escaping (Any?, String?) -> Void) {
        guard message.frameInfo.isMainFrame, let url = message.frameInfo.request.url, CanvasWebsiteController.isWebsiteURL(url),
              let body = message.body as? [String: Any] else { replyHandler(nil, "拒绝非内置主画布请求。"); return }
        Task { replyHandler(await handle(body), nil) }
    }
    func handle(_ payload: [String: Any]) async -> [String: Any] {
        do {
            if payload["operation"] as? String == "settings" { onSettings?(); return [:] }
            if payload["operation"] as? String == "export" {
                guard let name = payload["name"] as? String, !name.isEmpty, name.count <= 240,
                      !name.contains("/"), !name.contains("\\"), name != ".", name != "..",
                      !name.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
                      let body = payload["body"] as? String, body.utf8.count <= 280_000_000,
                      let data = Data(base64Encoded: body) else { throw DMXAIError.invalidInput }
                return ["saved": await onExport?(data, name) ?? false]
            }
            if payload["operation"] as? String == "video-submit" { return try await submitVideo(payload) }
            guard payload["operation"] as? String == "request", let path = payload["path"] as? String,
                  let method = payload["method"] as? String else { throw DMXAIError.invalidInput }
            if path == "/api/auth/verify", method == "GET" { return try jsonReply(["username": "本机画布", "role": "admin"]) }
            if path == "/api/dmx/v1/models", method == "GET" {
                return try jsonReply(["data": [["id": "gpt-image-2.5-sunburst"], ["id": "doubao-seedance-2-5-260628"], ["id": DMXAIClient.defaultTextModel]]])
            }
            if path.hasPrefix("/api/dmx/v1/videos/"), method == "GET" {
                return try await videoReply(path)
            }
            if path.hasPrefix("/api/dmx/media/"), method == "GET", let url = media[path] {
                return bytesReply(try await client.downloadCanvasMedia(url: url), type: "application/octet-stream")
            }
            let route = String(path.dropFirst("/api/dmx/v1/".count))
            guard path.hasPrefix("/api/dmx/v1/"), method == "POST",
                  ["images/generations", "images/edits", "chat/completions", "responses", "audio/speech"].contains(route),
                  let encoded = payload["body"] as? String, let bytes = Data(base64Encoded: encoded),
                  let type = payload["contentType"] as? String else { throw DMXAIError.invalidInput }
            let key = try await authorizedKey("将本次提示词及所选参考素材发送到 DMX。模型生成按你的 DMX 账户计费；软件不会自动重试。")
            let epoch = generationEpoch
            var result = try await client.canvasRequest(path: route, body: bytes, contentType: type, key: key)
            guard epoch == generationEpoch else { throw DMXAIError.cancelled }
            if route.hasPrefix("images/"), var json = (try? JSONSerialization.jsonObject(with: result)) as? [String: Any], var items = json["data"] as? [[String: Any]] {
                for index in items.indices {
                    if let address = items[index]["url"] as? String, let url = URL(string: address), url.scheme == "https" {
                        let local = "/api/dmx/media/" + UUID().uuidString
                        media[local] = url; items[index]["url"] = local
                    }
                }
                json["data"] = items; result = try JSONSerialization.data(withJSONObject: json)
            }
            let mime = route == "audio/speech" ? "application/octet-stream" : (String(data: result.prefix(20), encoding: .utf8)?.hasPrefix("data:") == true ? "text/event-stream" : "application/json")
            return bytesReply(result, type: mime)
        } catch {
            let message = (error as? DMXAIError)?.errorDescription ?? "本机钥匙串暂不可用；请重新设置 Key。"
            if payload["operation"] as? String == "video-submit" { return ["error": message] }
            return (try? jsonReply(["error": ["message": message]], status: 400)) ?? ["status": 400]
        }
    }
    private func authorizedKey(_ description: String) async throws -> String {
        guard let key = try readKey(), !key.isEmpty else { throw DMXAIError.invalidKey }
        let epoch = generationEpoch
        guard await confirmSend?(description) == true, epoch == generationEpoch else { throw DMXAIError.cancelled }
        return key
    }
    private func submitVideo(_ payload: [String: Any]) async throws -> [String: Any] {
        guard payload["model"] as? String == "doubao-seedance-2-5-260628", let prompt = payload["prompt"] as? String, !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw DMXAIError.invalidInput }
        var input: [[String: Any]] = [["type": "text", "text": prompt]]
        if let image = payload["image"] as? String {
            guard image.hasPrefix("data:image/") else { throw DMXAIError.invalidInput }
            input.append(["type": "image_url", "role": "first_frame", "image_url": ["url": image]])
        }
        let body: [String: Any] = ["model": "doubao-seedance-2-5-260628", "input": input,
            "duration": payload["duration"] ?? 4, "resolution": payload["resolution"] ?? "480p", "ratio": payload["ratio"] ?? "16:9",
            "generate_audio": payload["generateAudio"] ?? false, "watermark": payload["watermark"] ?? true]
        let bytes = try JSONSerialization.data(withJSONObject: body)
        let key = try await authorizedKey("提交 Seedance 2.5 视频生成，时长 \(payload["duration"] ?? 4) 秒。提示词及首帧图将发送到 DMX 并可能计费。取消本机等待不会取消远端任务。")
        // Return the submitted ID even if the UI changes while awaiting upstream; never silently resubmit.
        let result = try await client.canvasRequest(path: "responses", body: bytes, contentType: "application/json", key: key)
        guard let json = try JSONSerialization.jsonObject(with: result) as? [String: Any], let id = json["id"] as? String, DMXAIClient.validTaskID(id) else { throw DMXAIError.invalidResponse }
        return ["id": id]
    }
    private func videoReply(_ path: String) async throws -> [String: Any] {
        let parts = String(path.dropFirst("/api/dmx/v1/videos/".count)).split(separator: "/")
        guard let first = parts.first, DMXAIClient.validTaskID(String(first)), parts.count <= 2,
              parts.count == 1 || parts[1] == "content" else { throw DMXAIError.invalidInput }
        let id = String(first), local = "/api/dmx/v1/videos/" + id + "/content"
        if parts.count == 2, let url = media[local] { return bytesReply(try await client.downloadVideo(url: url), type: "video/mp4") }
        let key = try await authorizedKey("查询已有视频任务；只查询，不会重新提交生成。")
        let epoch = generationEpoch
        let state = try await client.queryVideo(taskID: id, key: key)
        guard epoch == generationEpoch else { throw DMXAIError.cancelled }
        switch state {
        case .running: return try jsonReply(["id": id, "status": "running"])
        case .failed: return try jsonReply(["id": id, "status": "failed"])
        case .completed(let url):
            media[local] = url
            if parts.count == 2 { return bytesReply(try await client.downloadVideo(url: url), type: "video/mp4") }
            return try jsonReply(["id": id, "status": "completed", "video_url": local])
        }
    }
    private func jsonReply(_ json: [String: Any], status: Int = 200) throws -> [String: Any] {
        var reply = bytesReply(try JSONSerialization.data(withJSONObject: json), type: "application/json"); reply["status"] = status; return reply
    }
    private func bytesReply(_ bytes: Data, type: String) -> [String: Any] { ["status": 200, "contentType": type, "body": bytes.base64EncodedString()] }
}
