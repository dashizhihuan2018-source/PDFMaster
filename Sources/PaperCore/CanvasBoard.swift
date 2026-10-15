import Foundation
import CoreGraphics

/// PDF Master's own portable board format. No Apple private data, remote resources,
/// executable actions, or external file references are read from this document.
public enum CanvasElementKind: String, Codable {
    case text, sticky, rectangle, ellipse, image, pdf, ink, connector
}

public struct CanvasElement: Identifiable, Codable, Equatable {
    public var id: UUID
    public var kind: CanvasElementKind
    public var frame: CGRect
    public var text: String
    public var strokeHex: String
    public var fillHex: String
    public var lineWidth: Double
    public var points: [CGPoint]
    public var assetData: Data?
    public var assetName: String?

    public init(id: UUID = UUID(), kind: CanvasElementKind, frame: CGRect,
                text: String = "", strokeHex: String = "#26334D", fillHex: String = "#FFF2AB",
                lineWidth: Double = 2, points: [CGPoint] = [], assetData: Data? = nil,
                assetName: String? = nil) {
        self.id = id; self.kind = kind; self.frame = frame; self.text = text
        self.strokeHex = strokeHex; self.fillHex = fillHex; self.lineWidth = lineWidth
        self.points = points; self.assetData = assetData; self.assetName = assetName
    }
}

public enum CanvasBoardError: Error, LocalizedError, Equatable {
    case unsupportedFormat, invalidGeometry, invalidStyle, duplicateElement
    case resourceLimit, invalidAsset, localFileRequired, invalidFile

    public var errorDescription: String? {
        switch self {
        case .unsupportedFormat: return "无法打开此文件，请选择 PDF Master 画布文件。"
        case .invalidGeometry: return "部分画布内容的位置或大小无效，无法完成此操作。"
        case .invalidStyle: return "颜色或描边宽度无效，请调整后重试。"
        case .duplicateElement: return "画布中的对象信息有冲突，请重新打开画布后重试。"
        case .resourceLimit: return "画布内容过多或附件过大，请减少对象数量或使用较小的文件。"
        case .invalidAsset: return "无法使用此附件，请重新选择有效的图片或 PDF 文件。"
        case .localFileRequired: return "请先将文件保存到这台 Mac，再选择该文件。"
        case .invalidFile: return "无法读取此画布文件，请检查文件是否完整且可以访问。"
        }
    }
}

public struct CanvasBoard: Codable, Equatable {
    public var id: UUID
    public var title: String
    public var elements: [CanvasElement]

    public init(id: UUID = UUID(), title: String = "未命名画布", elements: [CanvasElement] = []) {
        self.id = id; self.title = title; self.elements = elements
    }

    public static let maxFileBytes = 256 * 1_024 * 1_024
    public static let maxAssetBytes = 50 * 1_024 * 1_024
    private static let coordinateLimit: CGFloat = 1_000_000_000

    /// Validation never repairs or silently drops data: invalid input is rejected
    /// before it can replace the currently open board or enter undo history.
    public func validated() throws -> CanvasBoard {
        guard title.utf8.count <= 2_048, elements.count <= 10_000 else { throw CanvasBoardError.resourceLimit }
        var ids = Set<UUID>()
        var assetBytes = 0, pointCount = 0, textBytes = 0
        for element in elements {
            guard ids.insert(element.id).inserted else { throw CanvasBoardError.duplicateElement }
            let rect = element.frame
            // CGRect.width/height standardize a negative dimension; inspect the
            // raw size so malformed input is not silently admitted as positive.
            guard Self.valid(point: rect.origin), rect.size.width.isFinite, rect.size.height.isFinite,
                  rect.size.width > 0, rect.size.height > 0,
                  rect.size.width <= Self.coordinateLimit, rect.size.height <= Self.coordinateLimit,
                  Self.valid(point: CGPoint(x: rect.origin.x + rect.size.width, y: rect.origin.y + rect.size.height))
            else { throw CanvasBoardError.invalidGeometry }
            guard element.lineWidth.isFinite, (0.1...128).contains(element.lineWidth),
                  Self.valid(hex: element.strokeHex), Self.valid(hex: element.fillHex)
            else { throw CanvasBoardError.invalidStyle }
            guard element.points.count <= 200_000 else { throw CanvasBoardError.resourceLimit }
            for point in element.points where !Self.valid(point: point) { throw CanvasBoardError.invalidGeometry }
            pointCount += element.points.count
            textBytes += element.text.utf8.count
            guard pointCount <= 500_000, element.text.utf8.count <= 1_024 * 1_024,
                  textBytes <= 8 * 1_024 * 1_024 else { throw CanvasBoardError.resourceLimit }
            if let name = element.assetName {
                guard name.utf8.count <= 1_024 else { throw CanvasBoardError.resourceLimit }
            }
            if element.kind == .image || element.kind == .pdf {
                guard let asset = element.assetData, !asset.isEmpty else { throw CanvasBoardError.invalidAsset }
                guard asset.count <= Self.maxAssetBytes else { throw CanvasBoardError.resourceLimit }
                assetBytes += asset.count
                guard assetBytes <= Self.maxFileBytes else { throw CanvasBoardError.resourceLimit }
            } else if element.assetData != nil || element.assetName != nil {
                throw CanvasBoardError.invalidAsset
            }
        }
        return self
    }

    private static func valid(point: CGPoint) -> Bool {
        point.x.isFinite && point.y.isFinite && abs(point.x) <= coordinateLimit && abs(point.y) <= coordinateLimit
    }

    private static func valid(hex: String) -> Bool {
        let bytes = Array(hex.utf8)
        return (bytes.count == 7 || bytes.count == 9) && bytes.first == 35 &&
            bytes.dropFirst().allSatisfy { (48...57).contains($0) || (65...70).contains($0) || (97...102).contains($0) }
    }

    private struct Envelope: Codable {
        let format: String
        let version: Int
        let board: CanvasBoard
    }

    public func encoded() throws -> Data {
        _ = try validated()
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(Envelope(format: "PDFMasterBoard", version: 1, board: self))
        guard data.count <= Self.maxFileBytes else { throw CanvasBoardError.resourceLimit }
        return data
    }

    public static func decode(_ data: Data) throws -> CanvasBoard {
        guard !data.isEmpty else { throw CanvasBoardError.unsupportedFormat }
        guard data.count <= maxFileBytes else { throw CanvasBoardError.resourceLimit }
        do {
            let envelope = try JSONDecoder().decode(Envelope.self, from: data)
            guard envelope.format == "PDFMasterBoard", envelope.version == 1 else { throw CanvasBoardError.unsupportedFormat }
            return try envelope.board.validated()
        } catch let error as CanvasBoardError { throw error }
        catch { throw CanvasBoardError.unsupportedFormat }
    }

    private static func requireLocalFile(_ url: URL) throws {
        guard url.isFileURL, url.host == nil || url.host == "" || url.host == "localhost" else {
            throw CanvasBoardError.localFileRequired
        }
    }

    public func save(to url: URL) throws {
        try Self.requireLocalFile(url)
        // The atomic Foundation write preserves a previous valid file if a write
        // cannot be completed. It does not create folders or resolve board links.
        try encoded().write(to: url, options: .atomic)
    }

    public static func load(from url: URL) throws -> CanvasBoard {
        try requireLocalFile(url)
        let values = try url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey])
        guard values.isRegularFile == true, let size = values.fileSize, size > 0 else { throw CanvasBoardError.invalidFile }
        guard size <= maxFileBytes else { throw CanvasBoardError.resourceLimit }
        return try decode(Data(contentsOf: url, options: .mappedIfSafe))
    }
}

/// View coordinates = world coordinates * scale + offset. No page-sized
/// NSView or finite scroll document is needed for the infinite work surface.
public struct CanvasViewport: Equatable {
    public var scale: CGFloat {
        didSet {
            scale = scale.isFinite ? min(8, max(0.05, scale)) : oldValue
        }
    }
    public var offset: CGPoint {
        didSet {
            if !offset.x.isFinite || !offset.y.isFinite { offset = oldValue }
        }
    }

    public init(scale: CGFloat = 1, offset: CGPoint = .zero) {
        self.scale = scale.isFinite ? min(8, max(0.05, scale)) : 1
        self.offset = offset.x.isFinite && offset.y.isFinite ? offset : .zero
    }

    public func viewPoint(from point: CGPoint) -> CGPoint {
        CGPoint(x: point.x * scale + offset.x, y: point.y * scale + offset.y)
    }

    public func worldPoint(from point: CGPoint) -> CGPoint {
        CGPoint(x: (point.x - offset.x) / scale, y: (point.y - offset.y) / scale)
    }

    public mutating func zoom(to newScale: CGFloat, anchor: CGPoint) {
        guard newScale.isFinite, anchor.x.isFinite, anchor.y.isFinite else { return }
        let world = worldPoint(from: anchor)
        let clamped = min(8, max(0.05, newScale))
        let newOffset = CGPoint(x: anchor.x - world.x * clamped, y: anchor.y - world.y * clamped)
        guard newOffset.x.isFinite, newOffset.y.isFinite else { return }
        scale = clamped; offset = newOffset
    }

    public mutating func pan(by delta: CGSize) {
        let moved = CGPoint(x: offset.x + delta.width, y: offset.y + delta.height)
        guard moved.x.isFinite, moved.y.isFinite else { return }
        offset = moved
    }
}

@MainActor public final class CanvasSession {
    public private(set) var board: CanvasBoard
    public var onChange: (() -> Void)?
    private var savedBoard: CanvasBoard
    private var undoStack: [CanvasBoard] = []
    private var redoStack: [CanvasBoard] = []
    private let historyByteBudget = 256 * 1_024 * 1_024

    public init(board: CanvasBoard = CanvasBoard()) {
        let admitted = (try? board.validated()) ?? CanvasBoard()
        self.board = admitted; self.savedBoard = admitted
    }

    public var canUndo: Bool { !undoStack.isEmpty }
    public var canRedo: Bool { !redoStack.isEmpty }
    public var isDirty: Bool { board != savedBoard }

    // Conservative estimates intentionally count shared COW buffers per snapshot.
    // This can shorten history for large boards but never relies on shared buffers
    // continuing to share after subsequent asset or point edits.
    private func retainedBytes(_ board: CanvasBoard) -> Int {
        board.elements.reduce(board.title.utf8.count + 128) { total, element in
            total + (element.assetData?.count ?? 0) + element.points.count * MemoryLayout<CGPoint>.stride +
                element.text.utf8.count + (element.assetName?.utf8.count ?? 0) + 256
        }
    }

    private func trimHistory() {
        if undoStack.count > 100 { undoStack.removeFirst(undoStack.count - 100) }
        if redoStack.count > 100 { redoStack.removeFirst(redoStack.count - 100) }
        var bytes = (undoStack + redoStack).reduce(0) { $0 + retainedBytes($1) }
        while bytes > historyByteBudget {
            if !undoStack.isEmpty { bytes -= retainedBytes(undoStack.removeFirst()) }
            else if !redoStack.isEmpty { bytes -= retainedBytes(redoStack.removeFirst()) }
            else { break }
        }
    }

    public func replaceBoard(_ board: CanvasBoard) throws {
        let admitted = try board.validated()
        self.board = admitted; savedBoard = admitted
        undoStack.removeAll(); redoStack.removeAll()
        onChange?()
    }

    /// Call once at gesture end. Pointer-drag previews are held by CanvasView,
    /// not this model, so a stroke or a moved shape becomes exactly one undo.
    public func mutate(_ edit: (inout CanvasBoard) -> Void) throws {
        var candidate = board
        edit(&candidate)
        _ = try candidate.validated()
        guard candidate != board else { return }
        undoStack.append(board)
        board = candidate; redoStack.removeAll()
        trimHistory()
        onChange?()
    }

    public func undo() {
        guard let previous = undoStack.popLast() else { return }
        redoStack.append(board); board = previous; trimHistory(); onChange?()
    }

    public func redo() {
        guard let next = redoStack.popLast() else { return }
        undoStack.append(board); board = next; trimHistory(); onChange?()
    }

    public func markSaved() { savedBoard = board; onChange?() }

    public func save(to url: URL) throws {
        try board.save(to: url)
        markSaved()
    }
}
