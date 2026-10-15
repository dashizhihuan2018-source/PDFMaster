import Foundation

struct RecentDocumentEntry: Codable, Identifiable, Equatable {
    var id: String { normalizedURL.path }
    let normalizedURL: URL
    var lastOpened: Date?
    var isFavorite: Bool
    var exists: Bool { FileManager.default.fileExists(atPath: normalizedURL.path) }
}

final class RecentDocumentStore {
    let url: URL
    private(set) var entries: [RecentDocumentEntry] = []
    private(set) var lastError: Error?
    init(url: URL) { self.url = url; load() }

    func recordSuccessfulOpen(_ source: URL, at date: Date = Date()) throws {
        let normalized = Self.normalize(source)
        let favorite = entries.first { $0.normalizedURL == normalized }?.isFavorite ?? false
        var candidate = entries.filter { $0.normalizedURL != normalized }
        candidate.append(.init(normalizedURL: normalized, lastOpened: date, isFavorite: favorite))
        candidate = trimmed(candidate)
        try commit(candidate)
    }
    func toggleFavorite(_ source: URL) throws {
        let normalized = Self.normalize(source)
        var candidate = entries
        if let index = candidate.firstIndex(where: { $0.normalizedURL == normalized }) { candidate[index].isFavorite.toggle() }
        else { candidate.append(.init(normalizedURL: normalized, lastOpened: nil, isFavorite: true)) }
        try commit(trimmed(candidate))
    }
    func open(_ entry: RecentDocumentEntry, handler: (URL) -> Bool) throws -> Bool {
        guard entry.exists else { throw CocoaError(.fileNoSuchFile) }
        return handler(entry.normalizedURL)
    }
    func removeFromRecents(_ source: URL) throws {
        let normalized = Self.normalize(source)
        let candidate = entries.compactMap { item in
            guard item.normalizedURL == normalized else { return item }
            return item.isFavorite ? .init(normalizedURL: item.normalizedURL, lastOpened: nil, isFavorite: true) : nil
        }
        try commit(candidate)
    }
    func clearRecents() throws {
        let candidate = entries.compactMap { $0.isFavorite ? RecentDocumentEntry(normalizedURL: $0.normalizedURL, lastOpened: nil, isFavorite: true) : nil }
        try commit(candidate)
    }
    var recents: [RecentDocumentEntry] { entries.filter { $0.lastOpened != nil }.sorted { $0.lastOpened! > $1.lastOpened! } }
    var favorites: [RecentDocumentEntry] { entries.filter(\.isFavorite).sorted { ($0.lastOpened ?? .distantPast) > ($1.lastOpened ?? .distantPast) } }
    static func normalize(_ url: URL) -> URL { url.standardizedFileURL.resolvingSymlinksInPath() }
    private func trimmed(_ source: [RecentDocumentEntry]) -> [RecentDocumentEntry] {
        let keepRecent = Set(source.filter { !$0.isFavorite && $0.lastOpened != nil }.sorted { $0.lastOpened! > $1.lastOpened! }.prefix(50).map(\.normalizedURL))
        return source.filter { $0.isFavorite || keepRecent.contains($0.normalizedURL) }
    }
    private func load() {
        guard FileManager.default.fileExists(atPath: url.path) else { return }
        do { entries = trimmed(try JSONDecoder().decode([RecentDocumentEntry].self, from: Data(contentsOf: url))) }
        catch { lastError = error }
    }
    private func commit(_ candidate: [RecentDocumentEntry]) throws {
        do {
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try JSONEncoder().encode(candidate).write(to: url, options: .atomic)
            entries = candidate; lastError = nil
        } catch { lastError = error; throw error }
    }
}
