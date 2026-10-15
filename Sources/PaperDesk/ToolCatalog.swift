import AppKit

enum ToolGroup: String, CaseIterable, Codable {
    case edit = "编辑 PDF"
    case organize = "整理和优化"
    case toolbox = "PDF 工具箱"
    case convert = "转换 PDF"
    case export = "导出 PDF"
}

enum ToolAvailability: String, Codable { case available, partial, unavailable }

struct ToolDescriptor: Identifiable, Equatable {
    let id: String
    let title: String
    let group: ToolGroup
    let symbol: String
    let tint: NSColor
    let availability: ToolAvailability
    let unavailableReason: String?
}

enum ToolCatalog {
    static let all: [ToolDescriptor] = [
        tool("annotate", "批注", .edit, "highlighter", .systemYellow, .partial),
        tool("edit", "编辑", .edit, "text.cursor", .systemBlue, .partial),
        tool("sign", "签署 PDF", .edit, "signature", .systemPurple, .partial),
        tool("forms", "填写表单", .edit, "rectangle.and.pencil.and.ellipsis", .systemTeal, .unavailable),
        tool("comments", "评论 PDF", .edit, "text.bubble", .systemOrange, .partial),
        tool("resize", "页面重设大小", .edit, "arrow.up.left.and.arrow.down.right", .systemPink, .unavailable),
        tool("ocr", "识别文本", .organize, "viewfinder", .systemBlue, .partial),
        tool("pages", "管理页面", .organize, "square.grid.2x2", .systemIndigo, .partial),
        tool("compress", "压缩 PDF", .organize, "arrow.down.right.and.arrow.up.left", .systemGreen, .partial),
        tool("merge", "合并 PDF", .organize, "square.stack.3d.up", .systemOrange, .partial),
        tool("compare", "对比 PDF", .organize, "rectangle.split.2x1", .systemPink, .unavailable),
        tool("page-numbers", "页码", .organize, "number.square", .systemTeal, .partial),
        tool("copilot", "PDF Copilot", .toolbox, "sparkles", .systemPurple, .partial),
        tool("blank", "空白 PDF", .toolbox, "doc.badge.plus", .systemBlue, .available),
        tool("measure", "度量", .toolbox, "ruler", .systemOrange, .unavailable),
        tool("password", "设置密码", .toolbox, "lock", .systemGreen, .partial),
        tool("translate", "翻译 PDF", .toolbox, "character.book.closed", .systemIndigo, .unavailable),
        tool("redact", "隐藏 PDF 内容", .toolbox, "eye.slash", .systemRed, .partial),
        tool("redline", "红线", .toolbox, "line.diagonal", .systemRed, .unavailable),
        tool("draw", "绘图", .toolbox, "pencil.tip", .systemPurple, .partial),
        tool("image-to-pdf", "图片转 PDF", .convert, "photo", .systemBlue, .partial),
        tool("word-to-pdf", "Word 转 PDF", .convert, "doc.text", .systemIndigo, .unavailable),
        tool("sheet-to-pdf", "表转 PDF", .convert, "tablecells", .systemGreen, .unavailable),
        tool("file-to-pdf", "其他文件转 PDF", .convert, "doc.on.doc", .systemOrange, .unavailable),
        tool("pdf-to-word", "PDF 转 Word", .export, "doc.text.fill", .systemBlue, .unavailable),
        tool("pdf-to-image", "PDF 转图片", .export, "photo.on.rectangle", .systemPurple, .partial),
        tool("pdf-to-excel", "PDF 转 Excel", .export, "tablecells.fill", .systemGreen, .unavailable),
        tool("pdf-to-powerpoint", "PDF 转 PowerPoint", .export, "rectangle.on.rectangle", .systemOrange, .unavailable)
    ]

    static let defaultPreferredIDs = ["edit", "annotate", "pages", "measure", "sign", "copilot"]
    static func descriptor(id: String) -> ToolDescriptor? { all.first { $0.id == id } }
    private static func tool(_ id: String, _ title: String, _ group: ToolGroup, _ symbol: String, _ tint: NSColor, _ availability: ToolAvailability) -> ToolDescriptor {
        ToolDescriptor(id: id, title: title, group: group, symbol: symbol, tint: tint, availability: availability,
                       unavailableReason: availability == .unavailable ? "暂不支持此功能" : nil)
    }
}

final class PreferredToolStore {
    private struct Payload: Codable { let ids: [String] }
    let url: URL
    private(set) var toolIDs: [String]
    private(set) var lastError: Error?
    init(url: URL) {
        self.url = url
        guard FileManager.default.fileExists(atPath: url.path) else { toolIDs = ToolCatalog.defaultPreferredIDs; return }
        do { let payload = try JSONDecoder().decode(Payload.self, from: Data(contentsOf: url)); guard Self.valid(payload.ids) else { throw CocoaError(.validationMissingMandatoryProperty) }; toolIDs = payload.ids }
        catch { toolIDs = ToolCatalog.defaultPreferredIDs; lastError = error }
    }
    func replace(with ids: [String]) throws {
        do {
            guard Self.valid(ids) else { throw CocoaError(.validationMissingMandatoryProperty) }
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try JSONEncoder().encode(Payload(ids: ids)).write(to: url, options: .atomic)
            toolIDs = ids; lastError = nil
        } catch { lastError = error; throw error }
    }
    private static func valid(_ ids: [String]) -> Bool { ids.count == 6 && Set(ids).count == 6 && ids.allSatisfy { ToolCatalog.descriptor(id: $0) != nil } }
}
