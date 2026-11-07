import Foundation
import PDFKit
import CryptoKit

public enum PDFWatermarkKind: String, Codable, Hashable { case annotation, markedContent }

public struct PDFWatermarkCandidate: Codable, Equatable, Identifiable {
    public let id: String
    /// Zero-based page index. IDs are bound to the complete immutable input bytes.
    public let page: Int
    public let kind: PDFWatermarkKind
    public let evidence: String
}

public struct PDFWatermarkScan: Codable, Equatable {
    public let snapshotID: String
    public let pages: Set<Int>
    public let candidates: [PDFWatermarkCandidate]
    public let limitations: [String]
}

/// Lossless, explicit-object removal only. Never guesses from color, repeated
/// text, opacity or image pixels; no rasterization, covering or cloud service.
public final class PDFWatermarkEngine {
    private let qpdfOverride: URL?
    public init(qpdf: URL? = nil) { qpdfOverride = qpdf }
    private var qpdf: URL {
        if let qpdfOverride { return qpdfOverride }
        var candidates: [URL] = []
        if let resources = Bundle.main.resourceURL { candidates.append(resources.appendingPathComponent("Engines/qpdf/bin/qpdf")) }
        #if DEBUG
        if let path = ProcessInfo.processInfo.environment["PDFMASTER_TEST_QPDF"], !path.isEmpty { candidates.append(URL(fileURLWithPath: path)) }
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        candidates.append(root.appendingPathComponent("tools/qpdf-arm64-native-jpeg8-12.4.2/bin/qpdf"))
        #endif
        return candidates.first { FileManager.default.isExecutableFile(atPath: $0.path) } ?? URL(fileURLWithPath: "/unavailable/qpdf")
    }

    public func scan(_ source: URL, pages: Set<Int>, cancelled: @escaping () -> Bool) throws -> PDFWatermarkScan {
        let graph = try NativeContentGraph(input: source, qpdf: qpdf, cancelled: cancelled)
        return try inspect(graph, pages: pages).scan
    }

    /// Run on immutable admission bytes as well as the current PDFKit snapshot:
    /// serialization can otherwise omit a source signature or security catalog.
    public func validateAdmission(_ data: Data, in directory: URL, cancelled: @escaping () -> Bool) throws {
        let source = directory.appendingPathComponent("watermark-admission-\(UUID()).pdf")
        guard FileManager.default.createFile(atPath: source.path, contents: data, attributes: [.posixPermissions: 0o600]) else { throw OrganizeError.invalidInput }
        defer { try? FileManager.default.removeItem(at: source) }
        _ = try NativeContentGraph(input: source, qpdf: qpdf, cancelled: cancelled)
    }

    public func remove(_ source: URL, scan: PDFWatermarkScan, selectedIDs: Set<String>, output: URL, cancelled: @escaping () -> Bool) throws {
        guard !selectedIDs.isEmpty else { throw OrganizeError.processFailed("尚未选择水印候选；未生成副本") }
        // Refuse all existing outputs, including dangling symlinks and hardlinks.
        try Self.validateSeparateOutput(output, source: source)
        let graph = try NativeContentGraph(input: source, qpdf: qpdf, cancelled: cancelled)
        let fresh = try inspect(graph, pages: scan.pages)
        guard fresh.scan == scan, selectedIDs.isSubset(of: Set(scan.candidates.map(\.id))) else {
            throw OrganizeError.processFailed("水印候选已过期或不属于当前快照，请重新扫描")
        }
        let selected = fresh.locations.filter { selectedIDs.contains($0.candidate.id) }
        var nextObject = (graph.objects.keys.compactMap { Int($0.dropFirst(4).split(separator: " ").first ?? "") }.max() ?? 0) + 1
        for page in scan.pages.sorted() {
            if cancelled() { throw OrganizeError.cancelled }
            let edits = selected.filter { $0.candidate.page == page }
            guard !edits.isEmpty else { continue }
            let (reference, original) = try graph.pageDictionary(at: page)
            var dictionary = original
            let annotationIndices = Set(edits.compactMap(\.annotationIndex))
            if !annotationIndices.isEmpty {
                let annotations = try Self.annotations(graph, value: original["/Annots"])
                // Set a page-local array, never mutate a shared indirect array.
                dictionary["/Annots"] = annotations.enumerated().filter { !annotationIndices.contains($0.offset) }.map(\.element)
            }
            let streamEdits = edits.filter { $0.byteRange != nil }
            if !streamEdits.isEmpty {
                guard let streamReference = original["/Contents"] as? String else { throw OrganizeError.invalidInput }
                var data = try graph.stream(streamReference)
                for edit in streamEdits.sorted(by: { $0.byteRange!.lowerBound > $1.byteRange!.lowerBound }) {
                    data.replaceSubrange(edit.byteRange!, with: Data("\n".utf8))
                }
                let clone = "\(nextObject) 0 R"; nextObject += 1
                var streamDictionary = graph.dictionary(streamReference) ?? [:]
                for key in ["/Length", "/Filter", "/DecodeParms"] { streamDictionary.removeValue(forKey: key) }
                graph.setStream(clone, bytes: data, dictionary: streamDictionary)
                dictionary["/Contents"] = clone
            }
            graph.setObject(reference, value: dictionary)
        }
        try graph.rewrite(output: output)
    }

    public static func validateSeparateOutput(_ output: URL, source: URL?) throws {
        guard output.isFileURL, !output.hasDirectoryPath,
              output.standardizedFileURL.resolvingSymlinksInPath() != source?.standardizedFileURL.resolvingSymlinksInPath(),
              (try? FileManager.default.attributesOfItem(atPath: output.path)) == nil,
              !FileManager.default.fileExists(atPath: output.path) else { throw OrganizeError.outputExists }
    }

    private struct Location {
        let candidate: PDFWatermarkCandidate
        let annotationIndex: Int?
        let byteRange: Range<Int>?
    }
    private func inspect(_ graph: NativeContentGraph, pages: Set<Int>) throws -> (scan: PDFWatermarkScan, locations: [Location]) {
        guard !pages.isEmpty, pages.allSatisfy({ graph.pages.indices.contains($0) }) else { throw OrganizeError.invalidInput }
        let digest = SHA256.hash(data: try Data(contentsOf: graph.input)).map { String(format: "%02x", $0) }.joined()
        var locations: [Location] = [], limitations = ["仅识别显式 Watermark 批注及自包含 Artifact/Subtype Watermark 标记；扫描图、未标记文字/图片、复杂表单不支持。"]
        func candidate(_ page: Int, _ kind: PDFWatermarkKind, _ detail: String, _ evidence: String) -> PDFWatermarkCandidate {
            PDFWatermarkCandidate(id: NativeContentGraph.identity([digest, String(page), kind.rawValue, detail]), page: page, kind: kind, evidence: evidence)
        }
        for page in pages.sorted() {
            if graph.cancelled() { throw OrganizeError.cancelled }
            let (_, dictionary) = try graph.pageDictionary(at: page)
            let annotations = try Self.annotations(graph, value: dictionary["/Annots"])
            for (index, value) in annotations.enumerated() {
                guard let annotation = graph.dictionary(value) else { throw OrganizeError.invalidInput }
                if annotation["/Subtype"] as? String == "/Watermark" {
                    locations.append(Location(candidate: candidate(page, .annotation, "annotation-\(index)", "显式 /Subtype /Watermark 批注"), annotationIndex: index, byteRange: nil))
                }
            }
            guard dictionary["/Contents"] != nil else { continue }
            guard let reference = dictionary["/Contents"] as? String else {
                limitations.append("第 \(page + 1) 页：多内容流不提供标记删除候选（批注候选仍可选择）。"); continue
            }
            do {
                let data = try graph.stream(reference)
                for range in try Self.watermarkRanges(data) {
                    locations.append(Location(candidate: candidate(page, .markedContent, "\(reference)-\(range)", "显式 /Artifact << /Subtype /Watermark >> BDC…EMC；图形状态独立"), annotationIndex: nil, byteRange: range))
                }
            } catch {
                if graph.cancelled() { throw OrganizeError.cancelled }
                limitations.append("第 \(page + 1) 页：内容流结构复杂或不完整，不提供内容删除候选。")
            }
        }
        return (PDFWatermarkScan(snapshotID: digest, pages: pages, candidates: locations.map(\.candidate), limitations: limitations), locations)
    }
    private static func annotations(_ graph: NativeContentGraph, value: Any?) throws -> [Any] {
        guard let value else { return [] }
        if let array = value as? [Any] { return array }
        if let reference = value as? String, let object = graph.objects["obj:" + reference] as? [String: Any], let array = object["value"] as? [Any] { return array }
        throw OrganizeError.invalidInput
    }

    /// Deliberately narrow grammar: a flat literal property dictionary, isolated
    /// q…Q payload, no nested marked content, inline images or cross-stream state.
    private static func watermarkRanges(_ data: Data) throws -> [Range<Int>] {
        var lexer = NativePDFLexer(data), tokens: [NativePDFToken] = []
        while let token = try lexer.next() { tokens.append(token) }
        let supportedWords: Set<String> = [
            "q", "Q", "cm", "w", "J", "j", "M", "d", "ri", "i", "gs",
            "m", "l", "c", "v", "y", "h", "re", "S", "s", "f", "F", "f*", "B", "B*", "b", "b*", "n", "W", "W*",
            "BT", "ET", "Tc", "Tw", "Tz", "TL", "Tf", "Tr", "Ts", "Td", "TD", "Tm", "T*", "Tj", "TJ", "'", "\"",
            "CS", "cs", "SC", "SCN", "sc", "scn", "G", "g", "RG", "rg", "K", "k", "sh", "Do",
            "MP", "DP", "BMC", "BDC", "EMC", "<<", ">", "true", "false", "null"
        ]
        func word(_ index: Int) -> String? {
            guard tokens.indices.contains(index), case .word(let value) = tokens[index].value else { return nil }; return value
        }
        var marks: [(start: Int, payload: Int, eligible: Bool)] = [], ranges: [Range<Int>] = []
        // A PDF current path is NOT part of q/Q graphics state. Do not remove
        // a block that consumes or leaves a path belonging to adjacent body art.
        var graphics = 0, text = false, path = false, pendingClip = false
        for index in tokens.indices {
            guard let op = word(index) else { continue }
            guard supportedWords.contains(op) else { throw OrganizeError.invalidInput }
            switch op {
            case "BI", "ID", "EI": throw OrganizeError.invalidInput
            case "m", "l", "c", "v", "y", "h", "re": path = true
            case "W", "W*": pendingClip = true
            case "S", "s", "f", "F", "f*", "B", "B*", "b", "b*", "n": path = false; pendingClip = false
            case "q": guard !text else { throw OrganizeError.invalidInput }; graphics += 1
            case "Q": guard !text, graphics > 0 else { throw OrganizeError.invalidInput }; graphics -= 1
            case "BT": guard !text else { throw OrganizeError.invalidInput }; text = true
            case "ET": guard text else { throw OrganizeError.invalidInput }; text = false
            case "BMC": marks.append((index, index + 1, false))
            case "BDC":
                var start: Int?, eligible = false
                // NativePDFLexer intentionally emits >> as two '>' tokens.
                if index >= 6, word(index - 1) == ">", word(index - 2) == ">" {
                    var left = index - 3
                    while left > 0, word(left) != "<<", index - left < 40 { left -= 1 }
                    if word(left) == "<<", left > 0, tokens[left - 1].name == "/Artifact" {
                        let properties = Array(tokens[(left + 1)..<(index - 2)])
                        var names: [String: String] = [:], valid = properties.count % 2 == 0
                        if valid {
                            for offset in stride(from: 0, to: properties.count, by: 2) {
                                guard let key = properties[offset].name, let value = properties[offset + 1].name,
                                      ["/Subtype", "/Type"].contains(key), names[key] == nil else { valid = false; break }
                                names[key] = value
                            }
                        }
                        start = left - 1
                        eligible = valid && names["/Subtype"] == "/Watermark" && (names["/Type"] == nil || names["/Type"] == "/Pagination") && !text && !path && !pendingClip && marks.isEmpty
                    }
                }
                marks.append((start ?? index, index + 1, eligible))
            case "EMC":
                guard let mark = marks.popLast() else { throw OrganizeError.invalidInput }
                if mark.eligible, !path, !pendingClip, word(mark.payload) == "q", word(index - 1) == "Q" {
                    let payload = tokens[mark.payload..<index]
                    var depth = 0, safe = true
                    for (offset, token) in payload.enumerated() {
                        if case .word(let inner) = token.value {
                            if ["BDC", "BMC", "EMC"].contains(inner) { safe = false }
                            if inner == "q" { depth += 1 }
                            if inner == "Q" { depth -= 1 }
                            if depth == 0 && offset != payload.count - 1 { safe = false }
                            if depth < 0 { safe = false }
                        }
                    }
                    if safe && depth == 0 { ranges.append(tokens[mark.start].start..<tokens[index].end) }
                }
            default: break
            }
        }
        guard marks.isEmpty, graphics == 0, !text else { throw OrganizeError.invalidInput }
        return ranges
    }
}
