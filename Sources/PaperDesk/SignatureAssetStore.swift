import Foundation
import CryptoKit
import Darwin
import PaperCore

struct SignatureAsset: Codable, Identifiable, Equatable {
    let id: UUID
    let createdAt: Date
    let width: Int
    let height: Int
}

@MainActor final class SignatureAssetStore {
    private(set) var entries: [SignatureAsset] = []
    private(set) var lastError: Error?
    private let directory: URL
    private let limit = 128 * 1024 * 1024
    private enum StoreError: Error { case unsafeFile, corruptIndex, limit, missing, io }

    init(directory: URL) { self.directory = directory }
    private var index: URL { directory.appendingPathComponent("index.json") }
    private func assetURL(_ id: UUID) -> URL { directory.appendingPathComponent("\(id.uuidString).png") }

    private func perform<T>(_ body: () throws -> T) throws -> T {
        do { let result = try body(); lastError = nil; return result }
        catch { lastError = error; throw error }
    }

    private func prepare() throws {
        var info = stat()
        if lstat(directory.path, &info) != 0 {
            guard errno == ENOENT else { throw StoreError.io }
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            guard lstat(directory.path, &info) == 0 else { throw StoreError.io }
        }
        guard info.st_mode & S_IFMT == S_IFDIR, info.st_mode & 0o077 == 0 else { throw StoreError.unsafeFile }
    }

    private func read(_ url: URL, maximum: Int) throws -> Data {
        let fd = open(url.path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK)
        guard fd >= 0 else { throw StoreError.io }
        defer { close(fd) }
        var info = stat()
        guard fstat(fd, &info) == 0, info.st_mode & S_IFMT == S_IFREG,
              info.st_size >= 0, info.st_size <= maximum else { throw StoreError.unsafeFile }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 65536)
        while true {
            let count = Darwin.read(fd, &buffer, buffer.count)
            if count < 0 { if errno == EINTR { continue }; throw StoreError.io }
            if count == 0 { break }
            guard data.count <= maximum - count else { throw StoreError.limit }
            data.append(contentsOf: buffer.prefix(count))
        }
        return data
    }

    private func writeAtomic(_ data: Data, to url: URL) throws {
        let temp = directory.appendingPathComponent(".\(UUID().uuidString).tmp")
        let fd = open(temp.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
        guard fd >= 0 else { throw StoreError.io }
        defer { close(fd); unlink(temp.path) }
        try data.withUnsafeBytes { bytes in
            var offset = 0
            while offset < bytes.count {
                let n = Darwin.write(fd, bytes.baseAddress!.advanced(by: offset), bytes.count - offset)
                if n < 0 && errno == EINTR { continue }
                guard n > 0 else { throw StoreError.io }
                offset += n
            }
        }
        guard fsync(fd) == 0 else { throw StoreError.io }
        // Never replace a link or special file even though rename itself does not follow it.
        var existing = stat()
        if lstat(url.path, &existing) == 0 {
            guard existing.st_mode & S_IFMT == S_IFREG else { throw StoreError.unsafeFile }
        } else if errno != ENOENT { throw StoreError.io }
        let dirFD = open(directory.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW)
        guard dirFD >= 0 else { throw StoreError.io }
        defer { close(dirFD) }
        guard rename(temp.path, url.path) == 0 else { throw StoreError.io }
        // Once renamed the operation is committed: reporting failure here would
        // cause callers to roll back an asset referenced by the new index.
        _ = fsync(dirFD)
    }

    private func load() throws {
        try prepare()
        var info = stat()
        if lstat(index.path, &info) != 0 {
            guard errno == ENOENT else { throw StoreError.io }
            entries = []; return
        }
        let data = try read(index, maximum: 1024 * 1024)
        let decoded: [SignatureAsset]
        do { decoded = try JSONDecoder().decode([SignatureAsset].self, from: data) }
        catch { throw StoreError.corruptIndex }
        guard decoded.count <= 100, Set(decoded.map(\.id)).count == decoded.count,
              decoded.allSatisfy({ $0.width > 0 && $0.height > 0 && $0.width <= 4_000_000 / $0.height }) else { throw StoreError.corruptIndex }
        entries = decoded
    }
    func reload() throws { try perform { try load() } }

    func importPNG(from url: URL) throws -> SignatureAsset {
        try perform {
            try load()
            let image = try SignatureImage(pngData: read(url, maximum: 8 * 1024 * 1024))
            let hash = SHA256.hash(data: image.pngData)
            var total = 0
            for entry in entries {
                let bytes = try read(assetURL(entry.id), maximum: 8 * 1024 * 1024)
                total += bytes.count
                if SHA256.hash(data: bytes) == hash { return entry }
            }
            guard entries.count < 100, total <= limit - image.pngData.count else { throw StoreError.limit }
            let asset = SignatureAsset(id: UUID(), createdAt: Date(), width: image.pixelWidth, height: image.pixelHeight)
            let path = assetURL(asset.id)
            try writeAtomic(image.pngData, to: path)
            do { try writeAtomic(JSONEncoder().encode(entries + [asset]), to: index) }
            catch { unlink(path.path); throw error }
            entries.append(asset)
            return asset
        }
    }
    func image(for id: UUID) throws -> SignatureImage {
        try perform {
            guard entries.contains(where: { $0.id == id }) else { throw StoreError.missing }
            return try SignatureImage(pngData: read(assetURL(id), maximum: 8 * 1024 * 1024))
        }
    }
    func remove(id: UUID) throws {
        try perform {
            try load()
            guard entries.contains(where: { $0.id == id }) else { throw StoreError.missing }
            // Validate the asset before changing the index; unlink never follows a link.
            _ = try read(assetURL(id), maximum: 8 * 1024 * 1024)
            let remaining = entries.filter { $0.id != id }
            try writeAtomic(JSONEncoder().encode(remaining), to: index)
            entries = remaining
            guard unlink(assetURL(id).path) == 0 else { throw StoreError.io }
        }
    }
}
