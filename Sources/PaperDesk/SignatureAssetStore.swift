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
    private let limit: Int
    private let renameFile: (String, String) -> Int32
    private let unlinkFile: (String) -> Int32
    private enum StoreError: LocalizedError {
        case unsafeFile, corruptIndex, limit, missing, io, rollbackCleanup
        var errorDescription: String? {
            switch self {
            case .unsafeFile: return "签名文件或目录不安全。"
            case .corruptIndex: return "签名库索引损坏，未修改历史。"
            case .limit: return "签名库已达到容量限制。"
            case .missing: return "未找到该签名。"
            case .io: return "无法读写本机签名库。"
            case .rollbackCleanup: return "签名保存失败，临时资产清理失败，请检查本机签名库。"
            }
        }
    }

    init(directory: URL, byteLimit: Int = 128 * 1024 * 1024,
         renameFile: @escaping (String, String) -> Int32 = { Darwin.rename($0, $1) },
         unlinkFile: @escaping (String) -> Int32 = { Darwin.unlink($0) }) {
        self.directory = directory
        self.limit = max(0, min(byteLimit, 128 * 1024 * 1024))
        self.renameFile = renameFile
        self.unlinkFile = unlinkFile
    }
    private var index: URL { directory.appendingPathComponent("index.json") }
    private func assetURL(_ id: UUID) -> URL { directory.appendingPathComponent("\(id.uuidString).png") }

    private func perform<T>(_ body: () throws -> T) throws -> T {
        do { let result = try body(); lastError = nil; return result }
        catch { lastError = error; throw error }
    }

    private func prepare() throws {
        guard directory.isFileURL else { throw StoreError.unsafeFile }
        var info = stat()
        if lstat(directory.path, &info) != 0 {
            guard errno == ENOENT else { throw StoreError.io }
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            guard lstat(directory.path, &info) == 0 else { throw StoreError.io }
        }
        guard info.st_mode & S_IFMT == S_IFDIR, info.st_mode & 0o077 == 0 else { throw StoreError.unsafeFile }
    }

    private func read(_ url: URL, maximum: Int) throws -> Data {
        guard url.isFileURL else { throw StoreError.unsafeFile }
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
        defer { close(fd); _ = unlinkFile(temp.path) }
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
        guard renameFile(temp.path, url.path) == 0 else { throw StoreError.io }
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
        try save(draft(from: url))
    }

    /// Snapshot and normalize without touching the library. The safe reader is
    /// shared with stored assets; links, special files and oversized PNGs fail.
    func draft(from url: URL) throws -> SignatureImage {
        try perform { try SignatureImage(pngData: read(url, maximum: 8 * 1024 * 1024)) }
    }

    func save(_ draft: SignatureImage) throws -> SignatureAsset {
        try perform {
            try load()
            let image = try SignatureImage(pngData: draft.pngData)
            let hash = SHA256.hash(data: image.pngData)
            var total = 0
            for entry in entries {
                let url = assetURL(entry.id)
                var info = stat()
                // An absent library copy consumes no capacity. Keep its row in
                // history, but do not let it disable creation of healthy assets.
                // Existing links/special files still go through the safe reader.
                if lstat(url.path, &info) != 0 {
                    if errno == ENOENT { continue }
                    throw StoreError.io
                }
                let bytes = try read(url, maximum: 8 * 1024 * 1024)
                total += bytes.count
                if SHA256.hash(data: bytes) == hash { return entry }
            }
            guard entries.count < 100, total <= limit - image.pngData.count else { throw StoreError.limit }
            let asset = SignatureAsset(id: UUID(), createdAt: Date(), width: image.pixelWidth, height: image.pixelHeight)
            let path = assetURL(asset.id)
            try writeAtomic(image.pngData, to: path)
            do {
                try writeAtomic(JSONEncoder().encode(entries + [asset]), to: index)
            }
            catch {
                if unlinkFile(path.path) != 0 && errno != ENOENT { throw StoreError.rollbackCleanup }
                throw error
            }
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
            // A missing asset should not make its history entry impossible to remove.
            // Existing links and special files remain rejected, never followed.
            let path = assetURL(id)
            var info = stat()
            if lstat(path.path, &info) == 0 {
                _ = try read(path, maximum: 8 * 1024 * 1024)
            } else if errno != ENOENT { throw StoreError.io }
            let remaining = entries.filter { $0.id != id }
            try writeAtomic(JSONEncoder().encode(remaining), to: index)
            entries = remaining
            if unlinkFile(path.path) != 0 && errno != ENOENT { throw StoreError.io }
        }
    }
}
