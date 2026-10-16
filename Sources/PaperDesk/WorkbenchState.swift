import AppKit
import PDFKit
import PaperCore

public struct SearchSummary: Equatable {
    public let query: String
    public let resultCount: Int
    public let currentIndex: Int?

    public init(query: String, resultCount: Int, currentIndex: Int?) {
        self.query = query; self.resultCount = resultCount; self.currentIndex = currentIndex
    }

    public var text: String {
        if query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return "输入关键词搜索" }
        if resultCount == 0 { return "未找到“\(query)”" }
        return "\((currentIndex ?? 0) + 1) / \(resultCount)"
    }
}

public enum PageJump {
    public static func index(for text: String, pageCount: Int) -> Int? {
        guard let page = Int(text.trimmingCharacters(in: .whitespacesAndNewlines)),
              page >= 1, page <= pageCount else { return nil }
        return page - 1
    }
}

public enum WorkbenchMessage {
    public static func error(_ error: PDFWorkspaceError) -> String {
        switch error {
        case .noDocument: return "尚未打开 PDF 文档。"
        case .unreadableDocument: return "无法读取该 PDF，请检查文件是否完整。"
        case .lockedDocument: return "PDF 密码错误或文档无法解锁。"
        case .permissionDenied: return "文档权限不允许执行此操作。"
        case .invalidPage: return "页码无效，请输入现有页面。"
        case .invalidPageRange: return "页面范围无效，例如可输入 1-3,5。"
        case .cannotDeleteLastPage: return "不能删除文档的最后一页。"
        case .cannotOverwriteSource: return "为保护原文件，请选择其他位置保存副本。"
        case .sourceChanged: return "文件已在其他位置被修改、移动或替换。为保留双方更改，请另存为新文件，或重新打开原文件后继续。"
        case .invalidRotation: return "旋转角度无效。"
        case .invalidSelection: return "请先选择有效文字或页面区域。"
        case .invalidInk: return "手绘笔迹无效，请重新绘制。"
        case .staleEngineResult: return "处理期间文档已发生变化，旧结果未写回；请重新执行工具。"
        case .writeFailed: return "写入失败，请检查目标位置权限和可用空间。"
        }
    }
}

public enum DocumentAdmission {
    public static func hasInteractiveWidgets(_ document: PDFDocument) -> Bool {
        for index in 0..<document.pageCount {
            if document.page(at: index)?.annotations.contains(where: { $0.type == "Widget" }) == true {
                return true
            }
        }
        return false
    }

    /// Validates a candidate completely before the active workspace is replaced.
    /// The password only lives inside this synchronous call and is never persisted.
    @discardableResult
    public static func admit(
        url: URL,
        passwordPrompt: () -> String?,
        shouldReplace: () -> Bool,
        replace: (String?) throws -> Void
    ) throws -> Bool {
        guard let candidate = PDFDocument(url: url.standardizedFileURL) else {
            throw PDFWorkspaceError.unreadableDocument
        }

        var password: String?
        if candidate.isLocked {
            guard let supplied = passwordPrompt() else { return false }
            guard !supplied.isEmpty, candidate.unlock(withPassword: supplied) else {
                throw PDFWorkspaceError.lockedDocument
            }
            password = supplied
        }
        guard candidate.pageCount > 0 else { throw PDFWorkspaceError.unreadableDocument }
        guard !hasInteractiveWidgets(candidate) else { throw DocumentAdmissionError.unsupportedWidgets }
        guard shouldReplace() else { return false }
        try replace(password)
        return true
    }
}

public enum DocumentAdmissionError: Error, Equatable {
    case unsupportedWidgets
}

public enum ToolInputValidation {
    public static func rectangle(
        x: String,
        y: String,
        width: String,
        height: String,
        within containingBounds: CGRect? = nil
    ) throws -> CGRect {
        let values = [x, y, width, height].compactMap(parseFiniteNumber)
        guard values.count == 4 else { throw PDFWorkspaceError.invalidSelection }
        let rectangle = CGRect(x: values[0], y: values[1], width: values[2], height: values[3])
        guard rectangle.width > 0, rectangle.height > 0,
              containingBounds.map({ $0.contains(rectangle) }) ?? true else {
            throw PDFWorkspaceError.invalidSelection
        }
        return rectangle
    }

    public static func style(stroke: NSColor, fill: NSColor?, width: String) throws -> AnnotationStyle {
        guard let lineWidth = parseFiniteNumber(width), lineWidth > 0,
              stroke.usingColorSpace(.deviceRGB) != nil,
              fill?.usingColorSpace(.deviceRGB) != nil || fill == nil else {
            throw PDFWorkspaceError.invalidSelection
        }
        return AnnotationStyle(strokeColor: stroke, fillColor: fill, lineWidth: lineWidth)
    }

    public static func normalizedDrag(from start: CGPoint, to end: CGPoint, within bounds: CGRect) throws -> CGRect {
        guard [start.x, start.y, end.x, end.y].allSatisfy(\.isFinite) else {
            throw PDFWorkspaceError.invalidSelection
        }
        let rectangle = CGRect(
            x: min(start.x, end.x), y: min(start.y, end.y),
            width: abs(end.x - start.x), height: abs(end.y - start.y)
        )
        guard rectangle.width > 0, rectangle.height > 0, bounds.contains(rectangle) else {
            throw PDFWorkspaceError.invalidSelection
        }
        return rectangle
    }

    private static func parseFiniteNumber(_ text: String) -> CGFloat? {
        guard let value = Double(text.trimmingCharacters(in: .whitespacesAndNewlines)), value.isFinite else { return nil }
        return CGFloat(value)
    }
}

public struct SourceProtection {
    public let standardizedURL: URL
    public let resolvedURL: URL
    public let resourceIdentifier: AnyHashable?

    public static func capture(_ source: URL) -> SourceProtection? {
        let standardized = source.standardizedFileURL
        guard FileManager.default.fileExists(atPath: standardized.path) else { return nil }
        return SourceProtection(
            standardizedURL: standardized,
            resolvedURL: standardized.resolvingSymlinksInPath(),
            resourceIdentifier: resourceIdentifier(standardized)
        )
    }

    public func rejects(_ destination: URL) -> Bool {
        let output = destination.standardizedFileURL
        if output == standardizedURL || output.resolvingSymlinksInPath() == resolvedURL { return true }
        if let resourceIdentifier, let outputID = Self.resourceIdentifier(output), resourceIdentifier == outputID { return true }
        return false
    }

    private static func resourceIdentifier(_ url: URL) -> AnyHashable? {
        guard let values = try? url.resourceValues(forKeys: [.fileResourceIdentifierKey]),
              let identifier = values.fileResourceIdentifier as? AnyHashable else { return nil }
        return identifier
    }
}

public enum SafeTextExport {
    public static func write(_ text: String, to destination: URL, protection: SourceProtection?) throws {
        let output = destination.standardizedFileURL
        if protection?.rejects(output) == true { throw PDFWorkspaceError.cannotOverwriteSource }
        do { try Data(text.utf8).write(to: output, options: .atomic) }
        catch { throw PDFWorkspaceError.writeFailed }
    }
}

@MainActor
public enum PasswordProtectedCopyPolicy {
    public static func isAvailable(in workspace: PDFWorkspace) -> Bool {
        workspace.document != nil && !workspace.isEncryptedSource && workspace.allowsCopying
    }
}

public enum ExtractPagesProtection {
    public static let warning = "所选页面将导出为未加密的明文 PDF，原加密和保护不会保留。是否继续？"
}

public enum OutlineEditDisclosure {
    public static func information(removing: Bool) -> String {
        removing
            ? "删除顶层书签会同时删除其全部嵌套子项。此操作可通过“撤销”恢复。"
            : "仅重命名顶层项；其嵌套子项会保留。"
    }
}

public enum DocumentMenuSelectors {
    public static let copy = #selector(NSText.copy(_:))
    public static let cut = #selector(NSText.cut(_:))
    @MainActor static func addTextEditingItems(to menu: NSMenu) {
        for (title, action, key) in [("复制", copy, "c"), ("剪切", cut, "x"),
                                     ("粘贴", #selector(NSText.paste(_:)), "v"),
                                     ("粘贴并匹配样式", #selector(NSTextView.pasteAsPlainText(_:)), "V"),
                                     ("全选", #selector(NSText.selectAll(_:)), "a")] {
            // nil target preserves native field-editor and PDF permission routing.
            menu.addItem(withTitle: title, action: action, keyEquivalent: key)
        }
    }
}

public enum DocumentStatusPresentation {
    public static func message(hasDocument: Bool, isEncryptedSource: Bool) -> String {
        if hasDocument, isEncryptedSource {
            return "加密 PDF · 当前版本无法可靠判定或保留加密权限，打印与页面装配暂不支持"
        }
        return "PDF Master · 本地阅读与批注"
    }

    public static func toolTip(hasDocument: Bool, isEncryptedSource: Bool) -> String? {
        guard hasDocument, isEncryptedSource else { return nil }
        return "PDFKit 无法可靠判定并保留此加密来源的底层权限。为避免误放宽权限，当前版本保守禁用打印与页面装配；这不表示原文件本身必然禁止这些操作。"
    }
}

public enum DocumentOperationMessage {
    public static func printBlockReason(hasDocument: Bool, isEncryptedSource: Bool, allowsPrinting: Bool) -> String? {
        guard hasDocument else { return "尚未打开 PDF 文档。" }
        if isEncryptedSource {
            return "当前版本无法可靠判定或保留加密 PDF 的底层打印权限，因此暂不支持打印此来源。这不表示原文件的权限必然禁止打印。"
        }
        guard allowsPrinting else { return "文档的原始权限不允许打印。" }
        return nil
    }
}

public struct AnnotationSelectionReference: Equatable {
    public let page: Int
    public let index: Int
    public let revision: Int
    public let documentIdentity: ObjectIdentifier
}

@MainActor
public final class AnnotationSelectionTracker {
    public private(set) var selection: AnnotationSelectionReference?
    private weak var selectedDocument: PDFDocument?

    public init() {}

    public func select(page: Int, index: Int, in workspace: PDFWorkspace) {
        guard let document = workspace.document else { selection = nil; return }
        selectedDocument = document
        selection = AnnotationSelectionReference(
            page: page,
            index: index,
            revision: workspace.revision,
            documentIdentity: ObjectIdentifier(document)
        )
    }

    public func clear() { selection = nil }

    public func current(in workspace: PDFWorkspace) -> AnnotationSelectionReference? {
        guard let selection,
              selection.revision == workspace.revision,
              let document = workspace.document,
              selectedDocument === document,
              selection.documentIdentity == ObjectIdentifier(document),
              let page = document.page(at: selection.page),
              page.annotations.indices.contains(selection.index) else {
            self.selection = nil
            return nil
        }
        return selection
    }
}

public struct OutlineProjection: Equatable {
    public let label: String
    public let pageNumber: Int?
    public let depth: Int
    public let topLevelIndex: Int?
    public let path: [Int]

    public static func make(from document: PDFDocument) -> [OutlineProjection] {
        guard let root = document.outlineRoot else { return [] }
        var result: [OutlineProjection] = []

        func appendChildren(of parent: PDFOutline, depth: Int, path: [Int]) {
            for index in 0..<parent.numberOfChildren {
                guard let child = parent.child(at: index) else { continue }
                let childPath = path + [index]
                let pageNumber = child.destination?.page.flatMap { page in
                    let pageIndex = document.index(for: page)
                    return pageIndex == NSNotFound ? nil : pageIndex + 1
                }
                result.append(OutlineProjection(
                    label: child.label ?? "(未命名书签)",
                    pageNumber: pageNumber,
                    depth: depth,
                    topLevelIndex: depth == 0 ? index : nil,
                    path: childPath
                ))
                appendChildren(of: child, depth: depth + 1, path: childPath)
            }
        }
        appendChildren(of: root, depth: 0, path: [])
        return result
    }

    public static func outline(at path: [Int], in document: PDFDocument) -> PDFOutline? {
        guard var cursor = document.outlineRoot else { return nil }
        for component in path {
            guard let child = cursor.child(at: component) else { return nil }
            cursor = child
        }
        return cursor
    }
}

public enum SignaturePlacement {
    public static func map(strokes: [[CGPoint]], cropBox: CGRect, rotation: Int) -> [[CGPoint]] {
        let normalizedRotation = ((rotation % 360) + 360) % 360
        return strokes.map { stroke in
            stroke.map { point in
                let horizontal = (point.x - 0.5) * 160
                let vertical = (point.y - 0.5) * 70
                let offset: CGPoint
                switch normalizedRotation {
                case 90: offset = CGPoint(x: -vertical, y: horizontal)
                case 180: offset = CGPoint(x: -horizontal, y: -vertical)
                case 270: offset = CGPoint(x: vertical, y: -horizontal)
                default: offset = CGPoint(x: horizontal, y: vertical)
                }
                return CGPoint(x: cropBox.midX + offset.x, y: cropBox.midY + offset.y)
            }
        }
    }
}
