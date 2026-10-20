import Foundation
import WebKit
import UniformTypeIdentifiers

/// Read-only, confined asset root. No file:// access and no remote website fallback.
final class CanvasBundleScheme: NSObject, WKURLSchemeHandler {
    let root: URL
    static var resourceRoot: URL {
        let packaged = Bundle.main.resourceURL?.appendingPathComponent("CanvasWeb", isDirectory: true)
        if let packaged, FileManager.default.fileExists(atPath: packaged.appendingPathComponent("index.html").path) { return packaged }
        return URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("Resources/CanvasWeb", isDirectory: true)
    }
    init(root: URL = CanvasBundleScheme.resourceRoot) { self.root = root.resolvingSymlinksInPath().standardizedFileURL }
    func asset(for url: URL) -> URL? {
        guard url.scheme == "pdfmaster-canvas", url.host == "workspace", url.user == nil, url.password == nil, url.port == nil,
              !url.pathComponents.contains(".."), !url.path.contains("\\"), !url.path.hasPrefix("/api/") else { return nil }
        let file = root.appendingPathComponent(String(url.path.dropFirst())).resolvingSymlinksInPath().standardizedFileURL
        guard file.path.hasPrefix(root.path + "/") else { return url.path == "/" ? root.appendingPathComponent("index.html") : nil }
        if FileManager.default.fileExists(atPath: file.path), !file.hasDirectoryPath { return file }
        if url.pathExtension.isEmpty { return root.appendingPathComponent("index.html") }
        return nil
    }
    func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        guard let url = task.request.url, let file = asset(for: url), let data = try? Data(contentsOf: file) else { task.didFailWithError(URLError(.fileDoesNotExist)); return }
        let mime = file.pathExtension == "js" ? "application/javascript" : UTType(filenameExtension: file.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
        let headers = ["Content-Type": mime, "Content-Length": "\(data.count)",
            "Content-Security-Policy": "default-src 'self' data: blob:; script-src 'self' 'unsafe-inline' 'unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; media-src 'self' data: blob: https:; connect-src 'self' data: blob:; frame-src 'self' blob:; object-src 'none'; base-uri 'none'"]
        task.didReceive(HTTPURLResponse(url: url, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: headers)!)
        task.didReceive(data); task.didFinish()
    }
    func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {}
}
