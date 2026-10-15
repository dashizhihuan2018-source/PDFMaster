import AppKit
import Combine
import PDFKit
import CryptoKit

private final class PDFCatalogKeyCollector {
    var keys: Set<String> = []
}

private func collectPDFCatalogKey(
    _ key: UnsafePointer<CChar>,
    _ value: CGPDFObjectRef,
    _ context: UnsafeMutableRawPointer?
) {
    _ = value
    guard let context else { return }
    Unmanaged<PDFCatalogKeyCollector>.fromOpaque(context).takeUnretainedValue().keys.insert(String(cString: key))
}

public enum PDFWorkspaceError: Error, Equatable {
    case noDocument
    case unreadableDocument
    case lockedDocument
    case permissionDenied
    case invalidPage
    case invalidPageRange
    case cannotDeleteLastPage
    case cannotOverwriteSource
    case invalidRotation
    case invalidSelection
    case invalidInk
    case staleEngineResult
    case writeFailed
}

/// Recorded by the transaction itself, never inferred from page count or a
/// byte comparison. Assembly, catalog and engine changes require a full rebind.
public enum PDFHistoryPresentationChange: Equatable {
    case document
    case annotations(IndexSet)
}

public enum PageRange {
    public static func parse(_ text: String, count: Int) throws -> IndexSet {
        guard count > 0 else { throw PDFWorkspaceError.invalidPageRange }
        let pieces = text.split(separator: ",", omittingEmptySubsequences: false)
        guard !pieces.isEmpty else { throw PDFWorkspaceError.invalidPageRange }
        var result = IndexSet()
        for rawPiece in pieces {
            let piece = rawPiece.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !piece.isEmpty else { throw PDFWorkspaceError.invalidPageRange }
            let endpoints = piece.split(separator: "-", omittingEmptySubsequences: false)
            guard endpoints.count == 1 || endpoints.count == 2,
                  let first = Int(endpoints[0].trimmingCharacters(in: .whitespaces)),
                  (1...count).contains(first)
            else { throw PDFWorkspaceError.invalidPageRange }

            if endpoints.count == 1 {
                result.insert(first - 1)
            } else {
                guard let last = Int(endpoints[1].trimmingCharacters(in: .whitespaces)),
                      (1...count).contains(last)
                else { throw PDFWorkspaceError.invalidPageRange }
                for page in min(first, last)...max(first, last) {
                    result.insert(page - 1)
                }
            }
        }
        guard !result.isEmpty else { throw PDFWorkspaceError.invalidPageRange }
        return result
    }
}

public struct PDFEngineInput {
    public let data: Data
    /// Immutable admission bytes, before PDFKit can normalize catalog security keys.
    public let safetySourceData: Data
    public let safetySourceDatas: [Data]
    fileprivate let contentDigest: Data
    fileprivate let metadataDigest: Data
    fileprivate let generation: UUID
    fileprivate let revision: Int
}

@MainActor
public final class PDFWorkspace: ObservableObject {
    @Published public private(set) var document: PDFDocument?
    @Published public private(set) var sourceURL: URL?
    @Published public private(set) var isDirty = false
    @Published public private(set) var revision = 0

    public var canUndo: Bool { !undoSnapshots.isEmpty }
    public var canRedo: Bool { !redoSnapshots.isEmpty }
    public var allowsPrinting: Bool { permissions.printing }
    public var allowsCopying: Bool { permissions.copying }
    public var allowsCommenting: Bool { permissions.commenting }
    public var allowsDocumentAssembly: Bool { permissions.assembly }
    public var isEncryptedSource: Bool { sourceWasEncrypted }

    public func engineInput() throws -> PDFEngineInput {
        let current = try requireDocument()
        guard !sourceWasEncrypted else { throw PDFWorkspaceError.permissionDenied }
        try enforce(.copying, on: current)
        try enforce(.assembly, on: current)
        let metadata = try Self.metadataDigest(current.documentAttributes)
        guard let data = try serializedDocument(current), data.count <= 256 * 1024 * 1024 else {
            throw PDFWorkspaceError.unreadableDocument
        }
        return PDFEngineInput(data: data, safetySourceData: admittedContentInputs.first ?? data, safetySourceDatas: admittedContentInputs, contentDigest: Self.engineDigest(data), metadataDigest: metadata, generation: generation, revision: revision)
    }

    public func validateEngineInput(_ input: PDFEngineInput) throws {
        let current = try requireDocument()
        guard input.generation == generation, input.revision == revision,
              try Self.metadataDigest(current.documentAttributes) == input.metadataDigest,
              let bytes = try serializedDocument(current), Self.engineDigest(bytes) == input.contentDigest else { throw PDFWorkspaceError.staleEngineResult }
    }

    public func acceptEngineResult(_ data: Data, for input: PDFEngineInput) throws {
        let current = try requireDocument()
        guard !sourceWasEncrypted else { throw PDFWorkspaceError.permissionDenied }
        try enforce(.copying, on: current)
        try enforce(.assembly, on: current)
        guard input.generation == generation, input.revision == revision else {
            throw PDFWorkspaceError.staleEngineResult
        }
        guard try Self.metadataDigest(current.documentAttributes) == input.metadataDigest else { throw PDFWorkspaceError.staleEngineResult }
        guard let original = try serializedDocument(current) else { throw PDFWorkspaceError.unreadableDocument }
        guard Self.engineDigest(original) == input.contentDigest else { throw PDFWorkspaceError.staleEngineResult }
        guard data.count <= 256 * 1024 * 1024, let result = PDFDocument(data: data),
              !result.isEncrypted, !result.isLocked, (1...10000).contains(result.pageCount) else {
            throw PDFWorkspaceError.unreadableDocument
        }
        for index in 0..<result.pageCount {
            guard let page = result.page(at: index) else { throw PDFWorkspaceError.unreadableDocument }
            let bounds = page.bounds(for: .mediaBox)
            guard valid(bounds) else { throw PDFWorkspaceError.unreadableDocument }
        }
        redoSnapshots.removeAll()
        undoSnapshots.append(makeSnapshot(data: original, isDirty: isDirty))
        trimHistory()
        document = result
        isDirty = true
        revision += 1
    }

    /// PDFKit regenerates trailer /ID and Info CreationDate/ModDate on every write,
    /// even ignoring explicit live date attributes. Normalize only those generated
    /// values in their structurally located native-write objects. Live attributes
    /// are separately hashed, so explicit date and other metadata changes still stale.
    private static func engineDigest(_ data: Data) -> Data {
        var stable = data
        if let trailer = data.range(of: Data("\ntrailer\n".utf8), options: .backwards),
           data.count - trailer.lowerBound < 4096,
           let text = String(data: data[trailer.lowerBound...], encoding: .ascii),
           let regex = try? NSRegularExpression(pattern: "/ID\\s*\\[\\s*<([0-9A-Fa-f]{32})>\\s*<([0-9A-Fa-f]{32})>\\s*\\]"),
           let match = regex.firstMatch(in: text, range: NSRange(location: 0, length: text.utf16.count)) {
            for index in 1...2 {
                let range = match.range(at: index)
                stable.replaceSubrange((trailer.lowerBound + range.location)..<(trailer.lowerBound + NSMaxRange(range)), with: repeatElement(UInt8(48), count: range.length))
            }
        }
        if let infoRange = nativeInfoObjectRange(data) {
            normalizeNativeInfoDates(in: infoRange, original: data, stable: &stable)
        }
        return Data(SHA256.hash(data: stable))
    }

    private static func metadataDigest(_ attributes: [AnyHashable: Any]?) throws -> Data {
        var remainingNodes = 10000
        var remainingBytes = 4 * 1024 * 1024
        func consumeNode(depth: Int) throws {
            guard depth <= 32, remainingNodes > 0 else { throw PDFWorkspaceError.unreadableDocument }
            remainingNodes -= 1
        }
        func consumeBytes(_ count: Int) throws {
            guard count <= remainingBytes else { throw PDFWorkspaceError.unreadableDocument }
            remainingBytes -= count
        }
        func canonical(_ value: Any, depth: Int) throws -> Any {
            try consumeNode(depth: depth)
            if let date = value as? Date {
                return ["type": "date", "value": String(date.timeIntervalSince1970.bitPattern)]
            }
            if let string = value as? String {
                try consumeBytes(string.utf8.count)
                return ["type": "string", "value": string]
            }
            if let number = value as? NSNumber {
                return ["type": "number-" + String(cString: number.objCType), "value": number.stringValue]
            }
            if let bytes = value as? Data {
                try consumeBytes(bytes.count)
                return ["type": "data", "value": bytes.base64EncodedString()]
            }
            if let array = value as? NSArray {
                guard array.count <= remainingNodes else { throw PDFWorkspaceError.unreadableDocument }
                return try (0..<array.count).map { try canonical(array.object(at: $0), depth: depth + 1) }
            }
            if let dictionary = value as? NSDictionary {
                guard dictionary.count <= remainingNodes else { throw PDFWorkspaceError.unreadableDocument }
                guard let bridged = dictionary as? [AnyHashable: Any] else { throw PDFWorkspaceError.unreadableDocument }
                return try entries(bridged, depth: depth + 1)
            }
            throw PDFWorkspaceError.unreadableDocument
        }
        func entries(_ dictionary: [AnyHashable: Any], depth: Int) throws -> [[String: Any]] {
            try consumeNode(depth: depth)
            guard dictionary.count <= remainingNodes else { throw PDFWorkspaceError.unreadableDocument }
            var keyed: [String: Any] = [:]
            for (key, value) in dictionary {
                let name: String
                if let attribute = key.base as? PDFDocumentAttribute { name = attribute.rawValue }
                else if let string = key.base as? String { name = string }
                else { throw PDFWorkspaceError.unreadableDocument }
                guard keyed[name] == nil else { throw PDFWorkspaceError.unreadableDocument }
                try consumeBytes(name.utf8.count)
                keyed[name] = try canonical(value, depth: depth + 1)
            }
            return keyed.keys.sorted().map { ["key": $0, "value": keyed[$0]!] }
        }
        let bytes = try JSONSerialization.data(withJSONObject: entries(attributes ?? [:], depth: 0), options: [.sortedKeys])
        return Data(SHA256.hash(data: bytes))
    }

    /// Follow ONLY the final traditional xref's /Info reference. Malformed or
    /// xref-stream output receives no normalization (conservative stale rejection).
    private static func nativeInfoObjectRange(_ data: Data) -> Range<Int>? {
        guard let trailer = data.range(of: Data("\ntrailer\n".utf8), options: .backwards),
              data.count - trailer.lowerBound < 4096,
              let text = String(data: data[trailer.lowerBound...], encoding: .ascii),
              let referenceRE = try? NSRegularExpression(pattern: "/Info\\s+(\\d+)\\s+0\\s+R"),
              let reference = referenceRE.firstMatch(in: text, range: NSRange(location: 0, length: text.utf16.count)),
              let object = Int((text as NSString).substring(with: reference.range(at: 1))),
              let startRE = try? NSRegularExpression(pattern: "\\nstartxref\\n(\\d+)\\n%%EOF"),
              let start = startRE.firstMatch(in: text, range: NSRange(location: 0, length: text.utf16.count)),
              let xrefOffset = Int((text as NSString).substring(with: start.range(at: 1))),
              (0..<trailer.lowerBound).contains(xrefOffset),
              let xrefText = String(data: data[xrefOffset..<trailer.lowerBound], encoding: .ascii)
        else { return nil }
        let lines = xrefText.split(separator: "\n", omittingEmptySubsequences: true)
        guard lines.first == "xref" else { return nil }
        var index = 1
        var infoOffset: Int?
        var objectOffsets: [Int] = []
        while index < lines.count {
            let header = lines[index].split(whereSeparator: { $0 == " " || $0 == "\r" })
            guard header.count == 2, let first = Int(header[0]), let count = Int(header[1]),
                  first >= 0, count > 0, count <= 100000, first <= Int.max - (count - 1),
                  index + count < lines.count else { return nil }
            for entry in 0..<count {
                let fields = lines[index + 1 + entry].split(whereSeparator: { $0 == " " || $0 == "\r" })
                guard fields.count == 3, let offset = Int(fields[0]), let generation = Int(fields[1]) else { return nil }
                if fields[2] == "n" {
                    guard (0..<xrefOffset).contains(offset) else { return nil }
                    objectOffsets.append(offset)
                    if first + entry == object {
                        guard generation == 0 else { return nil }
                        infoOffset = offset
                    }
                } else if fields[2] != "f" { return nil }
            }
            index += count + 1
        }
        guard let offset = infoOffset,
              let end = objectOffsets.filter({ $0 > offset }).min() ?? Optional(xrefOffset),
              end > offset, end - offset <= 1024 * 1024,
              data[offset..<end].starts(with: Data("\(object) 0 obj\n".utf8)) else { return nil }
        return offset..<end
    }

    /// Lex strings/hex strings/arrays rather than matching date-looking text inside
    /// a Title or another value. Only top-level keys in the located Info dictionary.
    private static func normalizeNativeInfoDates(in range: Range<Int>, original: Data, stable: inout Data) {
        let bytes = Array(original[range])
        var index = 0, dictionaryDepth = 0, arrayDepth = 0
        func whitespace(_ value: UInt8) -> Bool { [0, 9, 10, 12, 13, 32].contains(value) }
        func delimiter(_ value: UInt8) -> Bool { whitespace(value) || [40,41,60,62,91,93,123,125,47,37].contains(value) }
        while index < bytes.count {
            if bytes[index] == 37 {
                while index < bytes.count && bytes[index] != 10 { index += 1 }
            } else if bytes[index] == 40 {
                var depth = 1; index += 1
                while index < bytes.count && depth > 0 {
                    if bytes[index] == 92 { index += 2; continue }
                    if bytes[index] == 40 { depth += 1 }
                    if bytes[index] == 41 { depth -= 1 }
                    index += 1
                }
            } else if bytes[index] == 60 {
                if index + 1 < bytes.count && bytes[index + 1] == 60 { dictionaryDepth += 1; index += 2 }
                else { index += 1; while index < bytes.count && bytes[index] != 62 { index += 1 }; index += 1 }
            } else if bytes[index] == 62 && index + 1 < bytes.count && bytes[index + 1] == 62 {
                dictionaryDepth -= 1; index += 2
            } else if bytes[index] == 91 { arrayDepth += 1; index += 1 }
            else if bytes[index] == 93 { arrayDepth -= 1; index += 1 }
            else if bytes[index] == 47 {
                let start = index + 1; index = start
                while index < bytes.count && !delimiter(bytes[index]) { index += 1 }
                let key = String(decoding: bytes[start..<index], as: UTF8.self)
                guard dictionaryDepth == 1, arrayDepth == 0, ["CreationDate", "ModDate"].contains(key) else { continue }
                var value = index
                while value < bytes.count && whitespace(bytes[value]) { value += 1 }
                guard value < bytes.count, bytes[value] == 40 else { continue }
                let contentStart = value + 1
                value = contentStart
                while value < bytes.count && bytes[value] != 41 && bytes[value] != 92 && bytes[value] != 40 { value += 1 }
                guard value < bytes.count, bytes[value] == 41,
                      let date = String(bytes: bytes[contentStart..<value], encoding: .ascii),
                      date.range(of: "^D:[0-9]{14}Z[0-9]{2}'[0-9]{2}'$", options: .regularExpression) != nil else { continue }
                stable.replaceSubrange((range.lowerBound + contentStart)..<(range.lowerBound + value), with: repeatElement(UInt8(48), count: value - contentStart))
            } else { index += 1 }
        }
    }

    private struct Snapshot {
        let data: Data
        let isDirty: Bool
        let sequence: UInt64
        let presentationChange: PDFHistoryPresentationChange
    }

    private static let defaultHistoryByteLimit = 256 * 1024 * 1024
    private static let defaultPageLimit = 10000

    private enum Permission {
        case assembly
        case commenting
        case copying
    }

    private struct PermissionState {
        let assembly: Bool
        let commenting: Bool
        let copying: Bool
        let printing: Bool
        let accessPermissions: UInt

        static let unrestricted = PermissionState(
            assembly: true,
            commenting: true,
            copying: true,
            printing: true,
            accessPermissions: 0xff
        )
    }

    private let historyLimit = 20
    private let historyByteLimit: Int
    private let pageLimit: Int
    private var undoSnapshots: [Snapshot] = []
    private var redoSnapshots: [Snapshot] = []
    private var nextSnapshotSequence: UInt64 = 0
    private var protectedSourceURL: URL?
    private var protectedSourceIdentifier: AnyHashable?
    private var permissions = PermissionState.unrestricted
    private var sourceWasEncrypted = false
    private var admittedContentInputs: [Data] = []
    private var generation = UUID()
    private struct OutlineEntry {
        var label: String
        var page: Int
        var point: CGPoint
        var zoom: CGFloat
        var children: [OutlineEntry]
    }
    public init() {
        historyByteLimit = Self.defaultHistoryByteLimit
        pageLimit = Self.defaultPageLimit
    }

    init(historyByteLimit: Int, pageLimit: Int = 10000) {
        self.historyByteLimit = max(0, historyByteLimit)
        self.pageLimit = max(1, pageLimit)
    }

    public func open(_ url: URL) throws {
        try open(url, password: nil)
    }

    public func open(_ url: URL, password: String) throws {
        guard !password.isEmpty else { throw PDFWorkspaceError.lockedDocument }
        try open(url, password: Optional(password))
    }

    private func open(_ url: URL, password: String?) throws {
        let normalized = url.standardizedFileURL
        let attributes = try FileManager.default.attributesOfItem(atPath: normalized.path)
        guard let size = attributes[.size] as? NSNumber, size.intValue <= 256 * 1024 * 1024 else { throw PDFWorkspaceError.unreadableDocument }
        let rawSource = try Data(contentsOf: normalized)
        guard rawSource.count <= 256 * 1024 * 1024, let loaded = PDFDocument(data: rawSource) else {
            throw PDFWorkspaceError.unreadableDocument
        }
        let wasEncrypted = loaded.isEncrypted
        if loaded.isLocked {
            guard let password, loaded.unlock(withPassword: password) else {
                throw PDFWorkspaceError.lockedDocument
            }
        }
        let copyData: Data?
        if password != nil {
            copyData = loaded.dataRepresentation(options: [
                PDFDocumentWriteOption.userPasswordOption: "",
                PDFDocumentWriteOption.ownerPasswordOption: ""
            ])
        } else {
            copyData = loaded.dataRepresentation()
        }
        guard loaded.pageCount > 0, let data = copyData, let copy = PDFDocument(data: data), !copy.isLocked else {
            throw PDFWorkspaceError.unreadableDocument
        }

        document = copy
        admittedContentInputs = [rawSource]
        generation = UUID()
        sourceURL = normalized
        protectedSourceURL = normalized.resolvingSymlinksInPath()
        protectedSourceIdentifier = resourceIdentifier(for: normalized)
        permissions = PermissionState(
            assembly: wasEncrypted ? false : loaded.allowsDocumentAssembly,
            commenting: loaded.allowsCommenting,
            copying: loaded.allowsCopying,
            printing: wasEncrypted ? false : loaded.allowsPrinting,
            accessPermissions: loaded.accessPermissions.rawValue
        )
        sourceWasEncrypted = wasEncrypted
        isDirty = false
        revision = 0
        undoSnapshots.removeAll()
        redoSnapshots.removeAll()
    }

    public func createBlank() throws {
        let fresh = PDFDocument()
        fresh.insert(try blankPage(), at: 0)
        document = fresh
        admittedContentInputs = []
        generation = UUID()
        sourceURL = nil
        protectedSourceURL = nil
        protectedSourceIdentifier = nil
        permissions = .unrestricted
        sourceWasEncrypted = false
        isDirty = false
        revision = 0
        undoSnapshots.removeAll()
        redoSnapshots.removeAll()
    }

    /// Admit a freshly generated, annotation-free PDF as a new unsaved document.
    /// This is not the file-opening path and never associates a source to overwrite.
    public func createUnsavedDocument(data: Data) throws {
        guard data.count <= 256 * 1024 * 1024, let fresh = PDFDocument(data: data),
              !fresh.isLocked, !fresh.isEncrypted, (1...500).contains(fresh.pageCount) else {
            throw PDFWorkspaceError.unreadableDocument
        }
        for index in 0..<fresh.pageCount {
            guard let page = fresh.page(at: index), page.annotations.isEmpty,
                  valid(page.bounds(for: .mediaBox)), valid(page.bounds(for: .cropBox)) else {
                throw PDFWorkspaceError.unreadableDocument
            }
        }
        document = fresh
        admittedContentInputs = [data]
        generation = UUID()
        sourceURL = nil; protectedSourceURL = nil; protectedSourceIdentifier = nil
        permissions = .unrestricted; sourceWasEncrypted = false
        isDirty = true; revision = 0
        undoSnapshots.removeAll(); redoSnapshots.removeAll()
    }

    /// Admit a passive embedded PDF as an independent editor copy. Unlike the
    /// generated-image admission, legitimate pre-existing comments are retained.
    /// Validate original bytes before PDFKit can normalize away security keys.
    public func createUnsavedPDFCopy(data: Data) throws {
        let fresh = try PassivePDFCopyAdmission.validate(data)
        guard fresh.allowsCopying else { throw PDFWorkspaceError.permissionDenied }
        let copyPermissions = PermissionState(
            assembly: fresh.allowsDocumentAssembly, commenting: fresh.allowsCommenting,
            copying: fresh.allowsCopying, printing: fresh.allowsPrinting,
            accessPermissions: fresh.accessPermissions.rawValue)
        document = fresh
        admittedContentInputs = [data]
        generation = UUID()
        sourceURL = nil; protectedSourceURL = nil; protectedSourceIdentifier = nil
        permissions = copyPermissions; sourceWasEncrypted = false
        isDirty = true; revision = 0
        undoSnapshots.removeAll(); redoSnapshots.removeAll()
    }

    public func createUnsavedFileMerge(_ batch: [Data]) throws {
        // Stage independently: malformed later inputs cannot replace a live document.
        // Use the ordinary insertion admission for EVERY position, not canvas admission.
        let staging = PDFWorkspace()
        try staging.createBlank()
        try staging.insertPDFData(batch, at: 0)
        guard let fresh = staging.document, fresh.pageCount > 1 else { throw PDFWorkspaceError.unreadableDocument }
        fresh.removePage(at: fresh.pageCount - 1)
        document = fresh; admittedContentInputs = staging.admittedContentInputs
        generation = UUID()
        sourceURL = nil; protectedSourceURL = nil; protectedSourceIdentifier = nil
        permissions = .unrestricted; sourceWasEncrypted = false
        isDirty = true; revision = 0
        undoSnapshots.removeAll(); redoSnapshots.removeAll()
    }

    public func saveCopy(to url: URL) throws {
        let current = try requireDocument()
        try enforce(.copying, on: current)
        guard !sourceWasEncrypted else { throw PDFWorkspaceError.permissionDenied }
        try validateOutputURL(url)
        guard let data = try serializedDocument(current) else { throw PDFWorkspaceError.writeFailed }
        do {
            try data.write(to: url.standardizedFileURL, options: .atomic)
        } catch {
            throw PDFWorkspaceError.writeFailed
        }
        isDirty = false
    }

    public func rotate(page: Int, degrees: Int) throws {
        guard degrees.isMultiple(of: 90) else { throw PDFWorkspaceError.invalidRotation }
        try mutate(requiring: .assembly) { candidate in
            guard let target = candidate.page(at: page) else { throw PDFWorkspaceError.invalidPage }
            let normalized = ((target.rotation + degrees) % 360 + 360) % 360
            target.rotation = normalized
        }
    }

    public func rotate(pages: IndexSet, degrees: Int) throws {
        guard degrees.isMultiple(of: 90) else { throw PDFWorkspaceError.invalidRotation }
        try mutate(requiring: .assembly) { candidate in
            try self.validate(pages: pages, count: candidate.pageCount)
            for index in pages {
                guard let page = candidate.page(at: index) else { throw PDFWorkspaceError.invalidPage }
                page.rotation = ((page.rotation + degrees % 360) % 360 + 360) % 360
            }
        }
    }

    /// Private in-app page clipboard. Never exports restricted/encrypted source content.
    public func copyPages(_ pages: IndexSet) throws -> Data {
        let current = try requireDocument()
        guard !sourceWasEncrypted else { throw PDFWorkspaceError.permissionDenied }
        try enforce(.copying, on: current); try enforce(.assembly, on: current)
        try validate(pages: pages, count: current.pageCount)
        let output = PDFDocument()
        for index in pages {
            guard let page = current.page(at: index), !page.annotations.contains(where: { $0.type == "Widget" || $0.type == "Link" }),
                  let copy = page.copy() as? PDFPage else { throw PDFWorkspaceError.invalidSelection }
            output.insert(copy, at: output.pageCount)
        }
        guard let data = output.dataRepresentation(), data.count <= 256 * 1024 * 1024 else { throw PDFWorkspaceError.writeFailed }
        return data
    }

    public func pastePages(_ data: Data, at index: Int) throws {
        try validateContentAdmissionBudget(data)
        guard data.count <= 256 * 1024 * 1024, let incoming = PDFDocument(data: data), !incoming.isEncrypted,
              !incoming.isLocked, incoming.pageCount > 0 else { throw PDFWorkspaceError.unreadableDocument }
        guard incoming.allowsCopying else { throw PDFWorkspaceError.permissionDenied }
        for i in 0..<incoming.pageCount {
            guard let page = incoming.page(at: i), !page.annotations.contains(where: { $0.type == "Widget" || $0.type == "Link" }) else { throw PDFWorkspaceError.invalidSelection }
        }
        try mutate(requiring: .assembly) { candidate in
            guard index >= 0, index <= candidate.pageCount, candidate.pageCount + incoming.pageCount <= 10000 else { throw PDFWorkspaceError.invalidPage }
            for offset in 0..<incoming.pageCount {
                guard let copy = incoming.page(at: offset)?.copy() as? PDFPage else { throw PDFWorkspaceError.unreadableDocument }
                candidate.insert(copy, at: index + offset)
            }
        }
        retainContentAdmission(data)
    }

    public func delete(pages: IndexSet) throws {
        try mutatePageStructure(requiring: .assembly) { candidate in
            try self.validate(pages: pages, count: candidate.pageCount)
            guard candidate.pageCount - pages.count >= 1 else {
                throw PDFWorkspaceError.cannotDeleteLastPage
            }
            let bookmarkTargets = try self.outlinePageIndexes(in: candidate)
            guard pages.isDisjoint(with: IndexSet(bookmarkTargets)) else {
                throw PDFWorkspaceError.invalidSelection
            }
            for index in pages.reversed() { candidate.removePage(at: index) }
            _ = try self.outlinePageIndexes(in: candidate)
        }
    }

    public func movePage(from: Int, to: Int) throws {
        try mutate(requiring: .assembly) { candidate in
            guard from >= 0, from < candidate.pageCount, to >= 0, to < candidate.pageCount,
                  let page = candidate.page(at: from)
            else { throw PDFWorkspaceError.invalidPage }
            _ = try self.outlinePageIndexes(in: candidate)
            candidate.removePage(at: from)
            candidate.insert(page, at: to)
            _ = try self.outlinePageIndexes(in: candidate)
        }
    }

    /// Move pages in their original relative order to an original-document
    /// insertion boundary (0...pageCount), as one undoable transaction.
    public func movePages(pages: IndexSet, to insertion: Int) throws {
        let current = try requireDocument()
        try enforce(.assembly, on: current)
        try validate(pages: pages, count: current.pageCount)
        guard (0...current.pageCount).contains(insertion) else { throw PDFWorkspaceError.invalidPage }
        _ = try outlinePageIndexes(in: current)

        // Dropping a contiguous block anywhere inside its own boundaries leaves
        // the order unchanged. Do not publish a clone or consume redo history.
        if let first = pages.first, let last = pages.last,
           pages.count == last - first + 1, (first...(last + 1)).contains(insertion) { return }
        let destination = insertion - pages.count(in: 0..<insertion)
        try mutatePageStructure(requiring: .assembly) { candidate in
            guard candidate.pageCount == current.pageCount else { throw PDFWorkspaceError.unreadableDocument }
            _ = try self.outlinePageIndexes(in: candidate)
            let movingPages = try pages.map { index -> PDFPage in
                guard let page = candidate.page(at: index) else { throw PDFWorkspaceError.invalidPage }
                return page
            }
            // Retain the original cloned page objects, including annotation and
            // outline destination references, instead of copying or rebuilding.
            for index in pages.reversed() { candidate.removePage(at: index) }
            for (offset, page) in movingPages.enumerated() {
                candidate.insert(page, at: destination + offset)
            }
        }
    }

    public func insertBlank(at index: Int) throws {
        try mutatePageStructure(requiring: .assembly) { candidate in
            guard index >= 0, index <= candidate.pageCount else { throw PDFWorkspaceError.invalidPage }
            _ = try self.outlinePageIndexes(in: candidate)
            candidate.insert(try self.blankPage(), at: index)
            _ = try self.outlinePageIndexes(in: candidate)
        }
    }

    public func insertPDFs(_ urls: [URL], at index: Int) throws {
        let current = try requireDocument()
        try enforce(.assembly, on: current)
        guard !urls.isEmpty, urls.count <= 256, index >= 0, index <= current.pageCount else { throw PDFWorkspaceError.invalidPage }
        var bytes: [Data] = []
        var total = 0
        for url in urls {
            guard url.isFileURL else { throw PDFWorkspaceError.unreadableDocument }
            let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
            guard let size = attributes[.size] as? NSNumber,
                  size.int64Value >= 0, size.int64Value <= 256 * 1024 * 1024 else { throw PDFWorkspaceError.unreadableDocument }
            let raw = try Data(contentsOf: url)
            guard raw.count <= 512 * 1024 * 1024 - total else { throw PDFWorkspaceError.unreadableDocument }
            total += raw.count; bytes.append(raw)
        }
        try insertPDFData(bytes, at: index)
    }

    /// Prepared mixed files use the same admission/history path as external PDFs.
    public func insertPDFData(_ batch: [Data], at index: Int) throws {
        let current = try requireDocument()
        try enforce(.assembly, on: current)
        guard !batch.isEmpty, batch.count <= 256, index >= 0, index <= current.pageCount,
              current.pageCount <= pageLimit else { throw PDFWorkspaceError.invalidPage }
        var inputs: [(Data, PDFDocument)] = []
        var admissionBytes = admittedContentInputs.reduce(0) { $0 + $1.count }
        var incomingCount = 0
        for raw in batch {
            try validateContentAdmissionBudget(raw)
            if !admittedContentInputs.contains(raw), !inputs.contains(where: { $0.0 == raw }) {
                guard raw.count <= 512 * 1024 * 1024 - admissionBytes else { throw PDFWorkspaceError.unreadableDocument }
                admissionBytes += raw.count
            }
            guard let incoming = PDFDocument(data: raw), incoming.pageCount > 0 else { throw PDFWorkspaceError.unreadableDocument }
            guard !incoming.isEncrypted else { throw PDFWorkspaceError.permissionDenied }
            guard !incoming.isLocked else { throw PDFWorkspaceError.lockedDocument }
            guard incoming.allowsCopying else { throw PDFWorkspaceError.permissionDenied }
            guard incoming.pageCount <= pageLimit - current.pageCount - incomingCount else { throw PDFWorkspaceError.invalidPage }
            incomingCount += incoming.pageCount
            inputs.append((raw, incoming))
        }
        // Validate the entire batch before copying pages or opening a transaction.
        // A bad later file cannot leave the first file partly merged.
        let pages = try inputs.flatMap { input in
            try (0..<input.1.pageCount).map { pageIndex -> PDFPage in
                guard let page = input.1.page(at: pageIndex)?.copy() as? PDFPage else { throw PDFWorkspaceError.unreadableDocument }
                return page
            }
        }
        try mutatePageStructure(requiring: .assembly) { candidate in
            for (offset, page) in pages.enumerated() { candidate.insert(page, at: index + offset) }
        }
        for input in inputs { retainContentAdmission(input.0) }
    }

    public func insertPDF(_ url: URL, at index: Int) throws {
        try insertPDFs([url], at: index)
    }

    private func validateContentAdmissionBudget(_ bytes: Data) throws {
        guard bytes.count <= 256 * 1024 * 1024,
              admittedContentInputs.reduce(0, { $0 + $1.count }) + bytes.count <= 512 * 1024 * 1024 else { throw PDFWorkspaceError.unreadableDocument }
    }
    private func retainContentAdmission(_ bytes: Data) {
        if !admittedContentInputs.contains(bytes) { admittedContentInputs.append(bytes) }
    }

    public func extract(pages: IndexSet, to url: URL) throws {
        let current = try requireDocument()
        try enforce(.copying, on: current)
        try validate(pages: pages, count: current.pageCount)
        try validateOutputURL(url)
        let output = PDFDocument()
        for (destinationIndex, sourceIndex) in pages.enumerated() {
            guard let page = current.page(at: sourceIndex)?.copy() as? PDFPage else {
                throw PDFWorkspaceError.unreadableDocument
            }
            output.insert(page, at: destinationIndex)
        }
        guard let data = output.dataRepresentation() else { throw PDFWorkspaceError.writeFailed }
        do {
            try data.write(to: url.standardizedFileURL, options: .atomic)
        } catch {
            throw PDFWorkspaceError.writeFailed
        }
    }

    public func addText(_ text: String, page: Int, bounds: CGRect) throws {
        guard !text.isEmpty, bounds.width > 0, bounds.height > 0 else {
            throw PDFWorkspaceError.invalidSelection
        }
        try mutate(requiring: .commenting, annotationPages: annotationPageScope(page)) { candidate in
            guard let target = candidate.page(at: page) else { throw PDFWorkspaceError.invalidPage }
            let annotation = PDFAnnotation(bounds: bounds, forType: .freeText, withProperties: nil)
            annotation.contents = text
            annotation.font = .systemFont(ofSize: 12)
            annotation.fontColor = .black
            annotation.color = .clear
            target.addAnnotation(annotation)
        }
    }

    public func addMarkup(selection: PDFSelection, kind: PDFAnnotationSubtype, color: NSColor) throws {
        guard [.highlight, .underline, .strikeOut].contains(kind) else {
            throw PDFWorkspaceError.invalidSelection
        }
        let current = try requireDocument()
        var selections: [(pageIndex: Int, bounds: CGRect)] = []
        for fragment in selection.selectionsByLine() {
            guard fragment.pages.count == 1, let page = fragment.pages.first else {
                throw PDFWorkspaceError.invalidSelection
            }
            let index = current.index(for: page)
            guard index != NSNotFound else { throw PDFWorkspaceError.invalidSelection }
            let bounds = fragment.bounds(for: page)
            guard !bounds.isEmpty else { continue }
            selections.append((index, bounds))
        }
        guard !selections.isEmpty else { throw PDFWorkspaceError.invalidSelection }

        try mutate(requiring: .commenting, annotationPages: IndexSet(selections.map(\.pageIndex))) { candidate in
            for item in selections {
                guard let page = candidate.page(at: item.pageIndex) else {
                    throw PDFWorkspaceError.invalidSelection
                }
                let annotation = PDFAnnotation(bounds: item.bounds, forType: kind, withProperties: nil)
                annotation.color = color
                annotation.quadrilateralPoints = [
                    NSValue(point: NSPoint(x: 0, y: item.bounds.height)),
                    NSValue(point: NSPoint(x: item.bounds.width, y: item.bounds.height)),
                    NSValue(point: NSPoint(x: 0, y: 0)),
                    NSValue(point: NSPoint(x: item.bounds.width, y: 0))
                ]
                page.addAnnotation(annotation)
            }
        }
    }

    public func addInk(points: [CGPoint], page: Int, color: NSColor, width: CGFloat, smoothed: Bool = false) throws {
        try addInk(strokes: [points], page: page, color: color, width: width, smoothed: smoothed)
    }

    public func addInk(strokes: [[CGPoint]], page: Int, color: NSColor, width: CGFloat, smoothed: Bool = false) throws {
        guard !strokes.isEmpty, width.isFinite, width > 0,
              strokes.allSatisfy({ $0.count >= 2 && $0.allSatisfy({ $0.x.isFinite && $0.y.isFinite }) }) else {
            throw PDFWorkspaceError.invalidInk
        }
        try mutate(requiring: .commenting, annotationPages: annotationPageScope(page)) { candidate in
            guard let target = candidate.page(at: page) else { throw PDFWorkspaceError.invalidPage }
            for points in strokes {
                if smoothed {
                    let absolutePath = InkStrokePath.make(points: points)
                    let absoluteBounds = absolutePath.boundingBoxOfPath.insetBy(dx: -width, dy: -width)
                    var offset = CGAffineTransform(translationX: -absoluteBounds.minX, y: -absoluteBounds.minY)
                    guard let localPath = absolutePath.copy(using: &offset) else { throw PDFWorkspaceError.invalidInk }
                    let path = InkStrokePath.bezier(localPath); path.lineWidth = width
                    let annotation = PDFAnnotation(bounds: absoluteBounds, forType: .ink, withProperties: nil)
                    annotation.color = color
                    let border = PDFBorder(); border.lineWidth = width; annotation.border = border
                    annotation.add(path); target.addAnnotation(annotation)
                    continue
                }
                let absoluteBounds = points.reduce(CGRect.null) { partial, point in
                    partial.union(CGRect(origin: point, size: .zero))
                }.insetBy(dx: -width, dy: -width)
                let path = NSBezierPath()
                path.move(to: NSPoint(x: points[0].x - absoluteBounds.minX, y: points[0].y - absoluteBounds.minY))
                for point in points.dropFirst() {
                    path.line(to: NSPoint(x: point.x - absoluteBounds.minX, y: point.y - absoluteBounds.minY))
                }
                path.lineWidth = width
                let annotation = PDFAnnotation(bounds: absoluteBounds, forType: .ink, withProperties: nil)
                annotation.color = color
                let border = PDFBorder()
                border.lineWidth = width
                annotation.border = border
                annotation.add(path)
                target.addAnnotation(annotation)
            }
        }
    }

    public func addPopupNote(_ contents: String, author: String?, page: Int, bounds: CGRect, color: NSColor) throws {
        guard !contents.isEmpty, valid(bounds), valid(color) else { throw PDFWorkspaceError.invalidSelection }
        try mutate(requiring: .commenting, annotationPages: annotationPageScope(page), normalizeNativeNotes: true) { candidate in
            guard let target = candidate.page(at: page) else { throw PDFWorkspaceError.invalidPage }
            let annotation = PDFAnnotation(bounds: bounds, forType: .text, withProperties: nil)
            annotation.contents = contents
            annotation.userName = author
            annotation.color = color
            target.addAnnotation(annotation)
        }
    }

    public func addShape(_ shape: AnnotationShape, page: Int, bounds: CGRect, style: AnnotationStyle) throws {
        guard valid(bounds), valid(style) else { throw PDFWorkspaceError.invalidSelection }
        try mutate(requiring: .commenting, annotationPages: annotationPageScope(page)) { candidate in
            guard let target = candidate.page(at: page) else { throw PDFWorkspaceError.invalidPage }
            guard target.bounds(for: .cropBox).contains(bounds) else { throw PDFWorkspaceError.invalidSelection }
            let subtype: PDFAnnotationSubtype = shape == .rectangle ? .square : .circle
            let annotation = PDFAnnotation(bounds: bounds, forType: subtype, withProperties: nil)
            apply(style, to: annotation)
            target.addAnnotation(annotation)
        }
    }

    public func addLine(from start: CGPoint, to end: CGPoint, page: Int, style: AnnotationStyle, endings: LineEndings) throws {
        guard valid(start), valid(end), start != end, valid(style) else { throw PDFWorkspaceError.invalidSelection }
        let padding = max(style.lineWidth, 1)
        let bounds = CGRect(x: min(start.x, end.x), y: min(start.y, end.y), width: abs(end.x - start.x), height: abs(end.y - start.y))
            .insetBy(dx: -padding, dy: -padding)
        guard valid(bounds) else { throw PDFWorkspaceError.invalidSelection }
        try mutate(requiring: .commenting, annotationPages: annotationPageScope(page)) { candidate in
            guard let target = candidate.page(at: page) else { throw PDFWorkspaceError.invalidPage }
            let crop = target.bounds(for: .cropBox)
            // CGRect.contains(point) excludes maxX/maxY; PDF page edges are
            // valid endpoints, and padding is clipped to the visible page.
            guard start.x >= crop.minX, start.x <= crop.maxX, start.y >= crop.minY, start.y <= crop.maxY,
                  end.x >= crop.minX, end.x <= crop.maxX, end.y >= crop.minY, end.y <= crop.maxY else {
                throw PDFWorkspaceError.invalidSelection
            }
            let clippedBounds = bounds.intersection(crop)
            guard valid(clippedBounds) else { throw PDFWorkspaceError.invalidSelection }
            let annotation = PDFAnnotation(bounds: clippedBounds, forType: .line, withProperties: nil)
            annotation.startPoint = CGPoint(x: start.x - clippedBounds.minX, y: start.y - clippedBounds.minY)
            annotation.endPoint = CGPoint(x: end.x - clippedBounds.minX, y: end.y - clippedBounds.minY)
            switch endings {
            case .none: annotation.startLineStyle = .none; annotation.endLineStyle = .none
            case .arrow: annotation.startLineStyle = .none; annotation.endLineStyle = .openArrow
            case .doubleArrow: annotation.startLineStyle = .openArrow; annotation.endLineStyle = .openArrow
            }
            apply(style, to: annotation)
            target.addAnnotation(annotation)
        }
    }

    /// Moves/resizes and restyles only Square, Circle or Line annotations in one
    /// undo transaction. Line endpoints are local to their bounds, so resizing
    /// scales them while retaining direction and each existing line ending.
    public func updateShape(page: Int, index: Int, bounds: CGRect, style: AnnotationStyle) throws {
        guard valid(bounds), valid(style) else { throw PDFWorkspaceError.invalidSelection }
        try mutate(requiring: .commenting, annotationPages: annotationPageScope(page)) { candidate in
            guard let target = candidate.page(at: page) else { throw PDFWorkspaceError.invalidPage }
            guard index >= 0, index < target.annotations.count,
                  target.bounds(for: .cropBox).contains(bounds) else { throw PDFWorkspaceError.invalidSelection }
            let annotation = target.annotations[index]
            guard ["Square", "Circle", "Line"].contains(annotation.type ?? "") else {
                throw PDFWorkspaceError.invalidSelection
            }
            if annotation.type == "Line" {
                let previous = annotation.bounds
                let start = annotation.startPoint
                let end = annotation.endPoint
                guard valid(previous), valid(start), valid(end), start != end,
                      start.x >= 0, start.x <= previous.width, start.y >= 0, start.y <= previous.height,
                      end.x >= 0, end.x <= previous.width, end.y >= 0, end.y <= previous.height else {
                    throw PDFWorkspaceError.invalidSelection
                }
                // Divide first to avoid overflowing for extremely small imported bounds.
                let scaledStart = CGPoint(x: (start.x / previous.width) * bounds.width,
                                          y: (start.y / previous.height) * bounds.height)
                let scaledEnd = CGPoint(x: (end.x / previous.width) * bounds.width,
                                        y: (end.y / previous.height) * bounds.height)
                guard valid(scaledStart), valid(scaledEnd), scaledStart != scaledEnd else {
                    throw PDFWorkspaceError.invalidSelection
                }
                annotation.bounds = bounds
                annotation.startPoint = scaledStart
                annotation.endPoint = scaledEnd
            } else {
                annotation.bounds = bounds
            }
            apply(style, to: annotation)
        }
    }

    /// Restyles native Ink only. Rebase annotation-local paths while preserving
    /// their absolute page coordinates, including every imported stroke.
    public func updateInkStyle(page: Int, index: Int, style: AnnotationStyle) throws {
        guard valid(style), style.fillColor == nil else { throw PDFWorkspaceError.invalidSelection }
        try mutate(requiring: .commenting, annotationPages: annotationPageScope(page)) { candidate in
            guard let target = candidate.page(at:page) else { throw PDFWorkspaceError.invalidPage }
            guard target.annotations.indices.contains(index) else { throw PDFWorkspaceError.invalidSelection }
            let annotation = target.annotations[index]
            guard annotation.type == "Ink", let paths = annotation.paths, !paths.isEmpty else { throw PDFWorkspaceError.invalidSelection }
            var absolutePaths:[NSBezierPath] = [], absoluteBounds = CGRect.null
            for path in paths {
                guard let absolute = path.copy() as? NSBezierPath else { throw PDFWorkspaceError.invalidInk }
                var offset = AffineTransform(); offset.translate(x:annotation.bounds.minX,y:annotation.bounds.minY)
                absolute.transform(using:offset)
                let b = absolute.cgPath.boundingBoxOfPath
                guard b.minX.isFinite, b.minY.isFinite, b.maxX.isFinite, b.maxY.isFinite else { throw PDFWorkspaceError.invalidInk }
                absoluteBounds = absoluteBounds.union(b); absolutePaths.append(absolute)
            }
            let bounds = absoluteBounds.insetBy(dx:-style.lineWidth,dy:-style.lineWidth)
            guard valid(bounds) else { throw PDFWorkspaceError.invalidInk }
            for path in paths { annotation.remove(path) }
            annotation.bounds = bounds
            for path in absolutePaths {
                var offset = AffineTransform(); offset.translate(x:-bounds.minX,y:-bounds.minY)
                path.transform(using:offset); path.lineWidth = style.lineWidth
                annotation.add(path)
            }
            apply(style,to:annotation)
        }
    }

    public func editAnnotation(page: Int, index: Int, contents: String?, bounds: CGRect?, color: NSColor?) throws {
        if let bounds, !valid(bounds) { throw PDFWorkspaceError.invalidSelection }
        if let color, !valid(color) { throw PDFWorkspaceError.invalidSelection }
        let isNote = document?.page(at: page)?.annotations.indices.contains(index) == true && document?.page(at: page)?.annotations[index].type == "Text"
        try mutate(requiring: .commenting, annotationPages: annotationPageScope(page), normalizeNativeNotes: isNote) { candidate in
            guard let target = candidate.page(at: page) else { throw PDFWorkspaceError.invalidPage }
            guard index >= 0, index < target.annotations.count else { throw PDFWorkspaceError.invalidSelection }
            let annotation = target.annotations[index]
            // Form fields and navigation objects require their dedicated editing
            // paths, not the generic commenting transaction.
            guard annotation.type != "Widget", annotation.type != "Link" else { throw PDFWorkspaceError.invalidSelection }
            if let contents { annotation.contents = contents }
            if let bounds { annotation.bounds = bounds }
            if let color { annotation.color = color }
        }
    }

    /// A bounded, passive, single-annotation PDF carrier. Only explicitly supported
    /// visual properties are reconstructed; arbitrary dictionaries/actions and
    /// source-page contents never enter the clipboard.
    public func copyAnnotation(page: Int, index: Int) throws -> Data {
        let current = try requireDocument()
        guard !sourceWasEncrypted else { throw PDFWorkspaceError.permissionDenied }
        try enforce(.copying, on: current)
        guard let source = current.page(at: page) else { throw PDFWorkspaceError.invalidPage }
        guard source.annotations.indices.contains(index) else { throw PDFWorkspaceError.invalidSelection }
        let annotation = try clipboardAnnotation(source.annotations[index])
        let carrier = PDFDocument(), carrierPage = PDFPage()
        let crop = source.bounds(for: .cropBox)
        guard valid(crop) else { throw PDFWorkspaceError.invalidSelection }
        carrierPage.setBounds(crop, for: .mediaBox)
        carrierPage.setBounds(crop, for: .cropBox)
        carrierPage.addAnnotation(annotation); carrier.insert(carrierPage, at: 0)
        guard let bytes = carrier.dataRepresentation(), bytes.count <= 8 * 1_024 * 1_024 else {
            throw PDFWorkspaceError.writeFailed
        }
        _ = try annotationClipboardCarrier(bytes)
        return bytes
    }

    /// Inserts an independent annotation in one commenting transaction. Position
    /// is translated into the target crop; oversized objects are rejected rather
    /// than clipped/rescaled, preserving local ink/line/markup geometry.
    @discardableResult public func pasteAnnotation(_ data: Data, page: Int) throws -> Int {
        let carrier = try annotationClipboardCarrier(data)
        let annotation = try clipboardAnnotation(carrier.page(at: 0)!.annotations[0])
        var inserted = 0
        try mutate(requiring: .commenting, annotationPages: annotationPageScope(page)) { candidate in
            guard let target = candidate.page(at: page) else { throw PDFWorkspaceError.invalidPage }
            let crop = target.bounds(for: .cropBox), original = annotation.bounds
            guard valid(crop), original.width <= crop.width, original.height <= crop.height else {
                throw PDFWorkspaceError.invalidSelection
            }
            var bounds = original
            bounds.origin.x = min(max(original.minX + 12, crop.minX), crop.maxX - original.width)
            bounds.origin.y = min(max(original.minY - 12, crop.minY), crop.maxY - original.height)
            annotation.bounds = bounds
            inserted = target.annotations.count
            target.addAnnotation(annotation)
        }
        return inserted
    }

    private func annotationClipboardCarrier(_ data: Data) throws -> PDFDocument {
        guard !data.isEmpty, data.count <= 8 * 1_024 * 1_024 else { throw PDFWorkspaceError.invalidSelection }
        let carrier = try PassivePDFCopyAdmission.validate(data)
        guard carrier.pageCount == 1, let page = carrier.page(at: 0) else {
            throw PDFWorkspaceError.invalidSelection
        }
        // PDFKit writes a Text note's companion Popup automatically. It is not
        // a second user annotation and is never reconstructed on the destination.
        let roots = page.annotations.filter { $0.type != "Popup" }
        guard roots.count == 1, let root = roots.first,
              page.annotations.count <= 2,
              page.annotations.allSatisfy({ $0 === root || (root.type == "Text" && $0.type == "Popup" && $0.action == nil) }) else {
            throw PDFWorkspaceError.invalidSelection
        }
        // The popup is discarded regardless of its native wrapper identity.
        // Passive admission above rejects active/unknown entries before this
        // normalization; only the freshly reconstructed root is ever pasted.
        for popup in page.annotations where popup !== root { page.removeAnnotation(popup) }
        _ = try clipboardAnnotation(page.annotations[0])
        return carrier
    }

    private func clipboardAnnotation(_ source: PDFAnnotation) throws -> PDFAnnotation {
        let allowed: Set<String> = ["Square", "Circle", "Line", "Ink", "Text", "FreeText", "Highlight", "Underline", "StrikeOut"]
        guard let type = source.type, allowed.contains(type), source.action == nil,
              valid(source.bounds), valid(source.color),
              source.value(forAnnotationKey: PDFAnnotationKey(rawValue: "/AA")) == nil,
              source.value(forAnnotationKey: PDFAnnotationKey(rawValue: "/A")) == nil else {
            throw PDFWorkspaceError.invalidSelection
        }
        let annotation = PDFAnnotation(bounds: source.bounds, forType: PDFAnnotationSubtype(rawValue: "/" + type), withProperties: nil)
        let style = AnnotationStyle(annotation: source)
        annotation.color = style.strokeColor; annotation.interiorColor = style.fillColor
        // Several PDFKit annotation writers discard colour alpha. Numeric
        // carrier-only metadata keeps this visual property lossless; unlike an
        // arbitrary source dictionary, these two bounded scalar keys are safe.
        for (key, fill) in [("/PDFMasterStrokeOpacity", false), ("/PDFMasterFillOpacity", true)] {
            let color = fill ? annotation.interiorColor : annotation.color
            guard let color else { continue }
            let alpha = (source.value(forAnnotationKey: PDFAnnotationKey(rawValue: key)) as? NSNumber)?.doubleValue ?? color.alphaComponent
            guard alpha.isFinite, (0...1).contains(alpha) else { throw PDFWorkspaceError.invalidSelection }
            if fill { annotation.interiorColor = color.withAlphaComponent(alpha) }
            else { annotation.color = color.withAlphaComponent(alpha) }
            annotation.setValue(NSNumber(value: alpha), forAnnotationKey: PDFAnnotationKey(rawValue: key))
        }
        annotation.contents = source.contents; annotation.userName = source.userName
        annotation.shouldDisplay = source.shouldDisplay; annotation.shouldPrint = source.shouldPrint
        if let border = source.border {
            guard border.lineWidth.isFinite, (0...1_000).contains(border.lineWidth) else { throw PDFWorkspaceError.invalidSelection }
            annotation.border = border.copy() as? PDFBorder
        }
        if type == "FreeText" {
            annotation.font = source.font; annotation.fontColor = source.fontColor; annotation.alignment = source.alignment
        }
        if type == "Text" { annotation.iconType = source.iconType }
        if type == "Line" {
            guard valid(source.startPoint), valid(source.endPoint) else { throw PDFWorkspaceError.invalidSelection }
            annotation.startPoint = source.startPoint; annotation.endPoint = source.endPoint
            annotation.startLineStyle = source.startLineStyle; annotation.endLineStyle = source.endLineStyle
        }
        if type == "Ink" {
            guard let paths = source.paths, !paths.isEmpty, paths.count <= 10_000 else { throw PDFWorkspaceError.invalidInk }
            var elements = 0
            for path in paths {
                elements += path.elementCount
                guard elements <= 100_000, let copied = path.copy() as? NSBezierPath,
                      valid(path.cgPath.boundingBoxOfPath.insetBy(dx: -0.01, dy: -0.01)) else { throw PDFWorkspaceError.invalidInk }
                annotation.add(copied)
            }
        }
        if ["Highlight", "Underline", "StrikeOut"].contains(type) {
            guard let points = source.quadrilateralPoints, !points.isEmpty, points.count <= 40_000,
                  points.count.isMultiple(of: 4), points.allSatisfy({ valid($0.pointValue) }) else {
                throw PDFWorkspaceError.invalidSelection
            }
            annotation.quadrilateralPoints = points
        }
        return annotation
    }

    public func removeAnnotation(page: Int, index: Int) throws {
        try mutate(requiring: .commenting, annotationPages: annotationPageScope(page)) { candidate in
            guard let target = candidate.page(at: page) else { throw PDFWorkspaceError.invalidPage }
            guard index >= 0, index < target.annotations.count else { throw PDFWorkspaceError.invalidSelection }
            let annotation = target.annotations[index]
            guard annotation.type != "Widget", annotation.type != "Link" else { throw PDFWorkspaceError.invalidSelection }
            target.removeAnnotation(annotation)
        }
    }

    public func duplicatePage(at page: Int, insertingAt destination: Int) throws {
        try mutate(requiring: .assembly) { candidate in
            _ = try self.outlinePageIndexes(in: candidate)
            guard let source = candidate.page(at: page), destination >= 0, destination <= candidate.pageCount,
                  let duplicate = source.copy() as? PDFPage else { throw PDFWorkspaceError.invalidPage }
            candidate.insert(duplicate, at: destination)
            _ = try self.outlinePageIndexes(in: candidate)
        }
    }

    public func cropPage(at page: Int, to rectangle: CGRect) throws {
        guard valid(rectangle) else { throw PDFWorkspaceError.invalidSelection }
        try mutate(requiring: .assembly) { candidate in
            guard let target = candidate.page(at: page) else { throw PDFWorkspaceError.invalidPage }
            let media = target.bounds(for: .mediaBox)
            guard media.contains(rectangle) else { throw PDFWorkspaceError.invalidSelection }
            target.setBounds(rectangle, for: .cropBox)
        }
    }

    public func addBookmark(label: String, page: Int) throws {
        guard !label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw PDFWorkspaceError.invalidSelection }
        let current = try requireDocument()
        guard let target = current.page(at: page) else { throw PDFWorkspaceError.invalidPage }
        var entries = try outlineEntries(in: current)
        entries.append(OutlineEntry(label: label, page: page, point: target.bounds(for: .cropBox).origin, zoom: 0, children: []))
        try replaceOutline(with: entries)
    }

    public func renameBookmark(at index: Int, label: String) throws {
        guard !label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw PDFWorkspaceError.invalidSelection }
        let current = try requireDocument()
        var entries = try outlineEntries(in: current)
        guard index >= 0, index < entries.count else { throw PDFWorkspaceError.invalidSelection }
        entries[index].label = label
        try replaceOutline(with: entries)
    }

    public func removeBookmark(at index: Int) throws {
        let current = try requireDocument()
        var entries = try outlineEntries(in: current)
        guard index >= 0, index < entries.count else { throw PDFWorkspaceError.invalidSelection }
        entries.remove(at: index)
        try replaceOutline(with: entries)
    }

    public func exportPlainText() throws -> String {
        let current = try requireDocument(); try enforce(.copying, on: current)
        return (0..<current.pageCount).compactMap { current.page(at: $0)?.string }.joined(separator: "\n\n")
    }

    public func exportAnnotationSummary(format: AnnotationSummaryFormat) throws -> String {
        let current = try requireDocument(); try enforce(.copying, on: current)
        let entries = (0..<current.pageCount).flatMap { pageIndex in
            current.page(at: pageIndex)?.annotations.map { (pageIndex + 1, $0.type ?? "Unknown", $0.contents ?? "", $0.userName ?? "") } ?? []
        }
        switch format {
        case .plainText:
            return entries.map { "Page \($0.0) | \($0.1) | \($0.2) | \($0.3)" }.joined(separator: "\n")
        case .markdown:
            return entries.map { "- Page \($0.0) | \(markdownEscape($0.1)) | \(markdownEscape($0.2)) | \(markdownEscape($0.3))" }.joined(separator: "\n")
        case .html:
            let rows = entries.map { "<tr><td>\($0.0)</td><td>\(htmlEscape($0.1))</td><td>\(htmlEscape($0.2))</td><td>\(htmlEscape($0.3))</td></tr>" }.joined()
            return "<!doctype html><html><body><table><thead><tr><th>Page</th><th>Type</th><th>Contents</th><th>Author</th></tr></thead><tbody>\(rows)</tbody></table></body></html>"
        }
    }

    public func exportAnnotationSummaryData(format: AnnotationSummaryFormat) throws -> Data {
        Data(try exportAnnotationSummary(format: format).utf8)
    }

    public func saveFlattenedCopy(to url: URL) throws {
        let current = try requireDocument(); try enforce(.copying, on: current); try validateOutputURL(url)
        guard let data = current.dataRepresentation(options: [PDFDocumentWriteOption.burnInAnnotationsOption: true]) else { throw PDFWorkspaceError.writeFailed }
        try write(data, to: url)
    }

    public func saveEncryptedCopy(to url: URL, userPassword: String, ownerPassword: String) throws {
        guard !userPassword.isEmpty, !ownerPassword.isEmpty else { throw PDFWorkspaceError.invalidSelection }
        let current = try requireDocument()
        try enforce(.copying, on: current)
        guard !sourceWasEncrypted else { throw PDFWorkspaceError.permissionDenied }
        try validateOutputURL(url)
        guard let data = current.dataRepresentation(options: [
            PDFDocumentWriteOption.userPasswordOption: userPassword,
            PDFDocumentWriteOption.ownerPasswordOption: ownerPassword,
            PDFDocumentWriteOption.accessPermissionsOption: NSNumber(value: permissions.accessPermissions)
        ]) else { throw PDFWorkspaceError.writeFailed }
        try write(data, to: url)
    }

    @discardableResult public func undo() -> PDFHistoryPresentationChange? {
        guard let snapshot = undoSnapshots.last,
              let current = document, let currentData = current.dataRepresentation(),
              let restored = PDFDocument(data: snapshot.data)
        else { return nil }
        undoSnapshots.removeLast()
        redoSnapshots.append(makeSnapshot(data: currentData, isDirty: isDirty, presentationChange: snapshot.presentationChange))
        trimHistory()
        document = restored
        isDirty = snapshot.isDirty
        revision += 1
        return snapshot.presentationChange
    }

    @discardableResult public func redo() -> PDFHistoryPresentationChange? {
        guard let snapshot = redoSnapshots.last,
              let current = document, let currentData = current.dataRepresentation(),
              let restored = PDFDocument(data: snapshot.data)
        else { return nil }
        redoSnapshots.removeLast()
        undoSnapshots.append(makeSnapshot(data: currentData, isDirty: isDirty, presentationChange: snapshot.presentationChange))
        trimHistory()
        document = restored
        isDirty = snapshot.isDirty
        revision += 1
        return snapshot.presentationChange
    }

    private func mutate(requiring permission: Permission, annotationPages: IndexSet? = nil, normalizeNativeNotes: Bool = false, _ operation: (PDFDocument) throws -> Void) throws {
        let current = try requireDocument()
        try enforce(permission, on: current)
        guard let originalData = current.dataRepresentation(), let candidate = PDFDocument(data: originalData) else {
            throw PDFWorkspaceError.unreadableDocument
        }
        try operation(candidate)
        let validated: PDFDocument
        if permission == .commenting && !normalizeNativeNotes {
            // Comment APIs already validate geometry/type/permissions. Validate the
            // detached graph once; avoid serializing and reparsing all pages again.
            guard candidate.pageCount == current.pageCount else { throw PDFWorkspaceError.writeFailed }
            for index in 0..<candidate.pageCount {
                guard let page = candidate.page(at: index), valid(page.bounds(for: .mediaBox)) else { throw PDFWorkspaceError.writeFailed }
            }
            validated = candidate
        } else {
            guard candidate.pageCount > 0, let data = candidate.dataRepresentation(),
                  let reparsed = PDFDocument(data: data), reparsed.pageCount == candidate.pageCount
            else { throw PDFWorkspaceError.writeFailed }
            validated = reparsed
        }

        redoSnapshots.removeAll()
        let presentationChange: PDFHistoryPresentationChange
        if permission == .commenting, let annotationPages { presentationChange = .annotations(annotationPages) }
        else { presentationChange = .document }
        undoSnapshots.append(makeSnapshot(data: originalData, isDirty: isDirty, presentationChange: presentationChange))
        trimHistory()
        document = validated
        isDirty = true
        revision += 1
    }

    /// Page insertion and deletion operate only on a detached, already parsed clone.
    /// Keep one full serialization for the undo snapshot and one full parse for
    /// transaction isolation, then validate the resulting page graph directly.
    /// Other mutation classes retain the stricter serialize/reparse round trip.
    private func mutatePageStructure(
        requiring permission: Permission,
        _ operation: (PDFDocument) throws -> Void
    ) throws {
        let current = try requireDocument()
        try enforce(permission, on: current)
        guard let originalData = current.dataRepresentation(),
              let candidate = PDFDocument(data: originalData) else {
            throw PDFWorkspaceError.unreadableDocument
        }

        try operation(candidate)
        try validatePageStructure(candidate)

        redoSnapshots.removeAll()
        undoSnapshots.append(makeSnapshot(data: originalData, isDirty: isDirty))
        trimHistory()
        document = candidate
        isDirty = true
        revision += 1
    }

    private func validatePageStructure(_ candidate: PDFDocument) throws {
        guard (1...pageLimit).contains(candidate.pageCount) else {
            throw PDFWorkspaceError.writeFailed
        }
        for index in 0..<candidate.pageCount {
            guard let page = candidate.page(at: index), valid(page.bounds(for: .mediaBox)) else {
                throw PDFWorkspaceError.writeFailed
            }
        }
        _ = try outlinePageIndexes(in: candidate)
    }

    private func outlinePageIndexes(in document: PDFDocument) throws -> [Int] {
        func flatten(_ entries: [OutlineEntry]) -> [Int] {
            entries.flatMap { [$0.page] + flatten($0.children) }
        }
        return flatten(try outlineEntries(in: document))
    }

    private func outlineEntries(in document: PDFDocument) throws -> [OutlineEntry] {
        guard let root = document.outlineRoot else { return [] }
        func read(_ outline: PDFOutline) throws -> OutlineEntry {
            guard let label = outline.label, let destination = outline.destination,
                  let page = destination.page, valid(destination.point), destination.zoom.isFinite
            else { throw PDFWorkspaceError.invalidSelection }
            let pageIndex = document.index(for: page)
            guard pageIndex != NSNotFound, pageIndex >= 0, pageIndex < document.pageCount else {
                throw PDFWorkspaceError.invalidSelection
            }
            var children: [OutlineEntry] = []
            for index in 0..<outline.numberOfChildren {
                guard let child = outline.child(at: index) else { throw PDFWorkspaceError.invalidSelection }
                children.append(try read(child))
            }
            return OutlineEntry(label: label, page: pageIndex, point: destination.point, zoom: destination.zoom, children: children)
        }
        return try (0..<root.numberOfChildren).map { index in
            guard let child = root.child(at: index) else { throw PDFWorkspaceError.invalidSelection }
            return try read(child)
        }
    }

    private func replaceOutline(with entries: [OutlineEntry]) throws {
        let current = try requireDocument()
        try enforce(.assembly, on: current)
        try ensureCatalogSupportsOutlineRebuild(current)
        guard let originalData = current.dataRepresentation() else { throw PDFWorkspaceError.unreadableDocument }
        let candidate = PDFDocument()
        candidate.documentAttributes = current.documentAttributes
        for index in 0..<current.pageCount {
            guard let page = current.page(at: index)?.copy() as? PDFPage else { throw PDFWorkspaceError.unreadableDocument }
            candidate.insert(page, at: index)
        }
        do {
            let root = PDFOutline()
            func make(_ entry: OutlineEntry) throws -> PDFOutline {
                guard let page = candidate.page(at: entry.page) else { throw PDFWorkspaceError.invalidSelection }
                let outline = PDFOutline()
                outline.label = entry.label
                let destination = PDFDestination(page: page, at: entry.point)
                destination.zoom = entry.zoom
                outline.destination = destination
                for child in entry.children { outline.insertChild(try make(child), at: outline.numberOfChildren) }
                return outline
            }
            for entry in entries { root.insertChild(try make(entry), at: root.numberOfChildren) }
            candidate.outlineRoot = root
        }
        guard let data = try serializedDocument(candidate), let validated = PDFDocument(data: data),
              validated.pageCount == current.pageCount else { throw PDFWorkspaceError.writeFailed }
        redoSnapshots.removeAll()
        undoSnapshots.append(makeSnapshot(data: originalData, isDirty: isDirty))
        trimHistory()
        document = validated
        isDirty = true
        revision += 1
    }

    private func ensureCatalogSupportsOutlineRebuild(_ document: PDFDocument) throws {
        guard let data = document.dataRepresentation(),
              let provider = CGDataProvider(data: data as CFData),
              let pdf = CGPDFDocument(provider), let catalog = pdf.catalog
        else { throw PDFWorkspaceError.unreadableDocument }
        let collector = PDFCatalogKeyCollector()
        CGPDFDictionaryApplyFunction(
            catalog,
            collectPDFCatalogKey,
            Unmanaged.passUnretained(collector).toOpaque()
        )
        let supported = Set(["Type", "Pages", "Outlines"])
        guard collector.keys.isSubset(of: supported) else { throw PDFWorkspaceError.invalidSelection }
    }

    private func requireDocument() throws -> PDFDocument {
        guard let document else { throw PDFWorkspaceError.noDocument }
        guard !document.isLocked else { throw PDFWorkspaceError.lockedDocument }
        return document
    }

    private func annotationPageScope(_ page: Int) -> IndexSet? {
        // Let the transaction's existing page check throw for a negative index;
        // do not construct an IndexSet before validation for invalid input.
        page >= 0 ? IndexSet(integer: page) : nil
    }

    private func enforce(_ permission: Permission, on document: PDFDocument) throws {
        _ = document
        let allowed: Bool
        switch permission {
        case .assembly: allowed = permissions.assembly
        case .commenting: allowed = permissions.commenting
        case .copying: allowed = permissions.copying
        }
        guard allowed else { throw PDFWorkspaceError.permissionDenied }
    }

    private func validate(pages: IndexSet, count: Int) throws {
        guard !pages.isEmpty, pages.allSatisfy({ $0 >= 0 && $0 < count }) else {
            throw PDFWorkspaceError.invalidPage
        }
    }

    private func validateOutputURL(_ url: URL) throws {
        guard url.isFileURL else { throw PDFWorkspaceError.writeFailed }
        let normalized = url.standardizedFileURL
        if let protectedSourceURL,
           normalized.resolvingSymlinksInPath() == protectedSourceURL {
            throw PDFWorkspaceError.cannotOverwriteSource
        }
        if let protectedSourceIdentifier,
           resourceIdentifier(for: normalized) == protectedSourceIdentifier {
            throw PDFWorkspaceError.cannotOverwriteSource
        }
    }

    private func resourceIdentifier(for url: URL) -> AnyHashable? {
        guard let values = try? url.resourceValues(forKeys: [.fileResourceIdentifierKey]),
              let identifier = values.fileResourceIdentifier as? AnyHashable else { return nil }
        return identifier
    }

    private func blankPage() throws -> PDFPage {
        let size = NSSize(width: 612, height: 792)
        let image = NSImage(size: size)
        image.lockFocus()
        NSColor.white.setFill()
        NSRect(origin: .zero, size: size).fill()
        image.unlockFocus()
        guard let page = PDFPage(image: image) else { throw PDFWorkspaceError.writeFailed }
        return page
    }

    private func valid(_ point: CGPoint) -> Bool {
        point.x.isFinite && point.y.isFinite
    }

    private func valid(_ rectangle: CGRect) -> Bool {
        rectangle.origin.x.isFinite && rectangle.origin.y.isFinite &&
            rectangle.width.isFinite && rectangle.height.isFinite &&
            rectangle.width > 0 && rectangle.height > 0
    }

    private func valid(_ color: NSColor) -> Bool {
        guard let rgb = color.usingColorSpace(.deviceRGB) else { return false }
        return [rgb.redComponent, rgb.greenComponent, rgb.blueComponent, rgb.alphaComponent].allSatisfy(\.isFinite)
    }

    private func valid(_ style: AnnotationStyle) -> Bool {
        style.lineWidth.isFinite && AnnotationStyle.supportedLineWidths.contains(style.lineWidth) &&
            valid(style.strokeColor) && (style.fillColor.map(valid) ?? true)
    }

    private func apply(_ style: AnnotationStyle, to annotation: PDFAnnotation) {
        annotation.color = style.strokeColor
        annotation.interiorColor = style.fillColor
        let border = annotation.border ?? PDFBorder()
        border.lineWidth = style.lineWidth
        annotation.border = border
    }

    private func markdownEscape(_ value: String) -> String {
        htmlEscape(value).replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "*", with: "\\*")
            .replacingOccurrences(of: "_", with: "\\_")
            .replacingOccurrences(of: "`", with: "\\`")
            .replacingOccurrences(of: "[", with: "\\[")
            .replacingOccurrences(of: "]", with: "\\]")
            .replacingOccurrences(of: "|", with: "\\|")
            .replacingOccurrences(of: "\n", with: " ")
    }

    private func htmlEscape(_ value: String) -> String {
        value.replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
            .replacingOccurrences(of: "\"", with: "&quot;")
            .replacingOccurrences(of: "'", with: "&#39;")
    }

    private func write(_ data: Data, to url: URL) throws {
        do { try data.write(to: url.standardizedFileURL, options: .atomic) }
        catch { throw PDFWorkspaceError.writeFailed }
    }

    private func serializedDocument(_ document: PDFDocument) throws -> Data? {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("PaperCore-\(UUID().uuidString).pdf")
        defer { try? FileManager.default.removeItem(at: url) }
        guard document.write(to: url) else { return nil }
        return try? Data(contentsOf: url)
    }

    private func makeSnapshot(data: Data, isDirty: Bool, presentationChange: PDFHistoryPresentationChange = .document) -> Snapshot {
        nextSnapshotSequence += 1
        return Snapshot(data: data, isDirty: isDirty, sequence: nextSnapshotSequence, presentationChange: presentationChange)
    }

    /// Undo and redo are two views over one history. Bound their combined retained
    /// bytes, evicting the oldest-created snapshot while preserving nearer history.
    private func trimHistory() {
        func byteCount() -> Int {
            undoSnapshots.reduce(0) { $0 + $1.data.count } +
                redoSnapshots.reduce(0) { $0 + $1.data.count }
        }

        while undoSnapshots.count + redoSnapshots.count > historyLimit || byteCount() > historyByteLimit {
            switch (undoSnapshots.first, redoSnapshots.first) {
            case let (undo?, redo?):
                if undo.sequence <= redo.sequence { undoSnapshots.removeFirst() }
                else { redoSnapshots.removeFirst() }
            case (.some, nil):
                undoSnapshots.removeFirst()
            case (nil, .some):
                redoSnapshots.removeFirst()
            case (nil, nil):
                return
            }
        }
    }
}
