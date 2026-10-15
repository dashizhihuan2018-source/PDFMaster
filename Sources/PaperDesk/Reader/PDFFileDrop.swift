import AppKit

/// Keep every local file in order. Format/content admission happens during import,
/// so an unsupported item produces an explanation instead of silently disappearing.
enum PDFFileDrop {
    static func urls(from pasteboard: NSPasteboard) -> [URL]? {
        guard let items = pasteboard.pasteboardItems, !items.isEmpty else { return nil }
        var urls: [URL] = []
        for item in items {
            guard let text = item.string(forType: .fileURL),
                  let url = URL(string: text), url.isFileURL else { return nil }
            urls.append(url.standardizedFileURL)
        }
        return urls
    }
}
