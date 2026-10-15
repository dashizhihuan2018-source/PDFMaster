import XCTest
import AppKit
import PDFKit
import Darwin
import CoreText
@testable import PaperCore

final class TextPDFImportTests: XCTestCase {
    @MainActor
    func testCopiedMergedChinesePagesRemainSearchableAfterWorkspaceSave() throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("text.txt")
        try Data("拖入中文资料".utf8).write(to: input)
        let batch = try FilePDFImport.prepare([input, input])
        let workspace = PDFWorkspace()
        try workspace.createUnsavedDocument(data: batch[0])
        try workspace.insertPDFData([batch[1]], at: 1)
        XCTAssertEqual(workspace.document?.page(at: 0)?.string, "拖入中文资料")
        XCTAssertEqual(workspace.document?.page(at: 1)?.string, "拖入中文资料")
        let output = directory.appendingPathComponent("saved.pdf")
        try workspace.saveCopy(to: output)
        let saved = try XCTUnwrap(PDFDocument(url: output))
        XCTAssertEqual(saved.pageCount, 2)
        XCTAssertTrue(saved.page(at: 0)?.string?.contains("拖入中文资料") == true)
        XCTAssertTrue(saved.page(at: 1)?.string?.contains("拖入中文资料") == true)
    }

    @MainActor
    func testRichNativeChineseRetainsExactUnicodeAfterMergeAndSave() throws {
        let directory = try temporaryDirectory()
        let phrase = "拖入中文资料 人口山水女子木日月 Searchable 123"
        let source = NSAttributedString(string: phrase, attributes: [.font: NSFont(name: "PingFangSC-Regular", size: 18)!])
        for (ext, type) in [("rtf", NSAttributedString.DocumentType.rtf), ("docx", .officeOpenXML)] {
            let input = directory.appendingPathComponent("source.\(ext)")
            try source.data(from: NSRange(location: 0, length: source.length), documentAttributes: [.documentType: type]).write(to: input)
            let converted = directory.appendingPathComponent("converted-\(ext).pdf")
            _ = try TextPDFImport.convert(input, to: converted)
            let raw = try Data(contentsOf: converted)
            let workspace = PDFWorkspace()
            try workspace.createBlank()
            try workspace.insertPDFData([raw, raw], at: 0)
            let output = directory.appendingPathComponent("saved-\(ext).pdf")
            try workspace.saveCopy(to: output)
            let saved = try XCTUnwrap(PDFDocument(url: output))
            for page in 0..<2 {
                XCTAssertTrue(saved.page(at: page)?.string?.contains(phrase) == true, "\(ext): \(saved.page(at: page)?.string ?? "nil")")
            }
        }
    }

    private func temporaryDirectory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("text-pdf-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return directory
    }

    func testPlainChineseIsSearchableOnAnA4Page() throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("中文.txt")
        let output = directory.appendingPathComponent("result.pdf")
        try Data("中文离线转换\nPDF Master searchable text 123".utf8).write(to: input)
        XCTAssertEqual(try TextPDFImport.convert(input, to: output), 1)
        let document = try XCTUnwrap(PDFDocument(url: output))
        XCTAssertEqual(document.pageCount, 1)
        XCTAssertTrue(document.string?.contains("中文离线转换") == true)
        XCTAssertTrue(document.string?.contains("searchable text 123") == true)
        let page = try XCTUnwrap(document.page(at: 0))
        XCTAssertEqual(page.bounds(for: .mediaBox).width, 595.276, accuracy: 0.01)
        XCTAssertEqual(page.bounds(for: .mediaBox).height, 841.89, accuracy: 0.01)
    }

    func testMarkdownIsLiteralSearchableTextAndLongInputHasEveryLine() throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("notes.MD")
        let output = directory.appendingPathComponent("result.pdf")
        let lines = (0..<700).map { "条目\($0) 离线分页文字 # Heading **literal**" }
        try Data(lines.joined(separator: "\n").utf8).write(to: input)
        let count = try TextPDFImport.convert(input, to: output)
        let document = try XCTUnwrap(PDFDocument(url: output))
        XCTAssertGreaterThan(count, 5)
        XCTAssertEqual(document.pageCount, count)
        let extracted = document.string ?? ""
        for index in 0..<700 { XCTAssertTrue(extracted.contains("条目\(index) "), "Missing line \(index)") }
        XCTAssertTrue(extracted.contains("# Heading **literal**"))
    }

    func testRTFNativeImporterRetainsSearchableChinese() throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("styled.rtf")
        let output = directory.appendingPathComponent("result.pdf")
        let source = NSAttributedString(string: "富文本中文标题\nRich text body", attributes: [.font: NSFont.boldSystemFont(ofSize: 18)])
        let data = try source.data(from: NSRange(location: 0, length: source.length), documentAttributes: [.documentType: NSAttributedString.DocumentType.rtf])
        try data.write(to: input)
        XCTAssertEqual(try TextPDFImport.convert(input, to: output), 1)
        let document = try XCTUnwrap(PDFDocument(url: output))
        XCTAssertTrue(document.string?.contains("富文本中文标题") == true)
        XCTAssertTrue(document.string?.contains("Rich text body") == true)
    }

    func testNativeDOCXFixtureRetainsSearchableChinese() throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("native.docx")
        let output = directory.appendingPathComponent("result.pdf")
        let source = NSAttributedString(string: "原生文档中文\nDOCX text body", attributes: [.font: NSFont.systemFont(ofSize: 14)])
        let data = try source.data(from: NSRange(location: 0, length: source.length), documentAttributes: [.documentType: NSAttributedString.DocumentType.officeOpenXML])
        XCTAssertTrue(data.starts(with: [0x50, 0x4b]))
        try data.write(to: input)
        XCTAssertEqual(try TextPDFImport.convert(input, to: output), 1)
        let document = try XCTUnwrap(PDFDocument(url: output))
        XCTAssertTrue(document.string?.contains("原生文档中文") == true)
        XCTAssertTrue(document.string?.contains("DOCX text body") == true)
    }

    func testUnsupportedFormatsNeverImportHTMLOrNetwork() throws {
        let directory = try temporaryDirectory()
        for name in ["page.html", "page.htm", "sheet.xlsx", "slides.pptx", "old.doc", "bundle.rtfd", "unknown.bin"] {
            let input = directory.appendingPathComponent(name)
            try Data("<html><img src='https://example.invalid/private'></html>".utf8).write(to: input)
            XCTAssertThrowsError(try TextPDFImport.convert(input, to: directory.appendingPathComponent("result.pdf"))) {
                XCTAssertEqual($0 as? ImportFailure, .invalidInput)
            }
        }
        XCTAssertThrowsError(try TextPDFImport.convert(URL(string: "https://example.invalid/data.txt")!, to: directory.appendingPathComponent("result.pdf"))) {
            XCTAssertEqual($0 as? ImportFailure, .invalidInput)
        }
    }

    func testCancellationLeavesNoOutputOrStage() throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("long.txt")
        let output = directory.appendingPathComponent("result.pdf")
        try Data(String(repeating: "取消分页测试\n", count: 2_000).utf8).write(to: input)
        var checks = 0
        XCTAssertThrowsError(try TextPDFImport.convert(input, to: output, cancelled: { checks += 1; return checks >= 8 })) {
            XCTAssertEqual($0 as? ImportFailure, .cancelled)
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: directory.path), ["long.txt"])
    }

    func testExistingOutputIsNeverOverwrittenIncludingSymlinks() throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("input.txt")
        try Data("safe text".utf8).write(to: input)
        let output = directory.appendingPathComponent("result.pdf")
        let sentinel = Data("existing untouched".utf8)
        try sentinel.write(to: output)
        XCTAssertThrowsError(try TextPDFImport.convert(input, to: output)) { XCTAssertEqual($0 as? ImportFailure, .outputExists) }
        XCTAssertEqual(try Data(contentsOf: output), sentinel)
        let symlink = directory.appendingPathComponent("link.pdf")
        try FileManager.default.createSymbolicLink(at: symlink, withDestinationURL: directory.appendingPathComponent("missing.pdf"))
        XCTAssertThrowsError(try TextPDFImport.convert(input, to: symlink)) { XCTAssertEqual($0 as? ImportFailure, .outputExists) }
    }

    func testSymlinkDirectoryAndFIFOInputsAreRejectedWithoutBlocking() throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("input.txt")
        try Data("safe text".utf8).write(to: input)
        let symlink = directory.appendingPathComponent("linked.txt")
        try FileManager.default.createSymbolicLink(at: symlink, withDestinationURL: input)
        let fifo = directory.appendingPathComponent("pipe.txt")
        XCTAssertEqual(mkfifo(fifo.path, 0o600), 0)
        let folder = directory.appendingPathComponent("folder.txt")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
        for item in [symlink, fifo, folder] {
            XCTAssertThrowsError(try TextPDFImport.convert(item, to: directory.appendingPathComponent("result.pdf"))) {
                XCTAssertEqual($0 as? ImportFailure, .invalidInput)
            }
        }
    }

    func testInputByteAndPageLimitsAreBoundedWithoutPublishedOutput() throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("large.txt")
        let output = directory.appendingPathComponent("result.pdf")
        try Data(repeating: 65, count: 16 * 1024 * 1024 + 1).write(to: input)
        XCTAssertThrowsError(try TextPDFImport.convert(input, to: output)) { XCTAssertEqual($0 as? ImportFailure, .tooLarge) }
        try Data(String(repeating: "页数上限\n", count: 70_000).utf8).write(to: input)
        XCTAssertThrowsError(try TextPDFImport.convert(input, to: output)) { XCTAssertEqual($0 as? ImportFailure, .tooLarge) }
        XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: directory.path), ["large.txt"])
    }

    func testInvalidUTF8AndInvalidRTFAreDecodeFailures() throws {
        let directory = try temporaryDirectory()
        for (name, data) in [("invalid.txt", Data([0xff, 0xfe, 0xff])), ("invalid.rtf", Data("not an RTF document".utf8))] {
            let input = directory.appendingPathComponent(name)
            try data.write(to: input)
            XCTAssertThrowsError(try TextPDFImport.convert(input, to: directory.appendingPathComponent("result.pdf"))) {
                XCTAssertEqual($0 as? ImportFailure, .decodeFailed)
            }
        }
    }

    func testCancellationAfterStageCreationRemovesPrivateStage() throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("input.txt")
        let output = directory.appendingPathComponent("result.pdf")
        try Data("取消已建立的暂存文件".utf8).write(to: input)
        var sawStage = false
        XCTAssertThrowsError(try TextPDFImport.convert(input, to: output, cancelled: {
            sawStage = (try? FileManager.default.contentsOfDirectory(atPath: directory.path).contains { $0.hasPrefix(".text-import-") }) == true
            return sawStage
        })) { XCTAssertEqual($0 as? ImportFailure, .cancelled) }
        XCTAssertTrue(sawStage)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: directory.path), ["input.txt"])
    }

    func testOutputCreatedAfterPreflightIsNotOverwritten() throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("input.txt")
        let output = directory.appendingPathComponent("result.pdf")
        let sentinel = Data("racing output stays unchanged".utf8)
        try Data("安全合并文字".utf8).write(to: input)
        var created = false
        XCTAssertThrowsError(try TextPDFImport.convert(input, to: output, cancelled: {
            if !created, (try? FileManager.default.contentsOfDirectory(atPath: directory.path).contains { $0.hasPrefix(".text-import-") }) == true {
                created = true
                try? sentinel.write(to: output)
            }
            return false
        })) { XCTAssertEqual($0 as? ImportFailure, .outputExists) }
        XCTAssertTrue(created)
        XCTAssertEqual(try Data(contentsOf: output), sentinel)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: directory.path).sorted(), ["input.txt", "result.pdf"])
    }

    func testUTF16BOMChineseAndEmptyTextProduceValidSearchablePDFs() throws {
        let directory = try temporaryDirectory()
        for (name, data) in [("utf16.txt", "中文双字节编码".data(using: .utf16)!), ("empty.txt", Data())] {
            let input = directory.appendingPathComponent(name)
            let output = directory.appendingPathComponent("\(name).pdf")
            try data.write(to: input)
            XCTAssertEqual(try TextPDFImport.convert(input, to: output), 1)
            let document = try XCTUnwrap(PDFDocument(url: output))
            XCTAssertEqual(document.pageCount, 1)
            if name == "utf16.txt" { XCTAssertTrue(document.string?.contains("中文双字节编码") == true) }
        }
    }

    func testRTFAttachmentsAreRejectedInsteadOfSilentlyLost() throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("attachment.rtf")
        let png = try XCTUnwrap(Data(base64Encoded: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aJZkAAAAASUVORK5CYII="))
        let hex = png.map { String(format: "%02x", $0) }.joined()
        let data = Data("{\\rtf1\\ansi Text with image {\\pict\\pngblip\\picw1\\pich1 \(hex)}}".utf8)
        try data.write(to: input)
        XCTAssertThrowsError(try TextPDFImport.convert(input, to: directory.appendingPathComponent("result.pdf"))) {
            XCTAssertEqual($0 as? ImportFailure, .invalidInput)
        }
    }

    func testImportWorksOnBackgroundWorker() async throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("background.rtf")
        let source = NSAttributedString(string: "后台原生转换中文")
        try source.data(from: NSRange(location: 0, length: source.length), documentAttributes: [.documentType: NSAttributedString.DocumentType.rtf]).write(to: input)
        let result = try await LocalToolJob.run(data: Data(), cancellation: LocalToolCancellation()) { _, output, flag in
            XCTAssertFalse(Thread.isMainThread)
            return "\(try TextPDFImport.convert(input, to: output, cancelled: { flag.isCancelled }))"
        }
        XCTAssertEqual(result.message, "1")
        let document = try XCTUnwrap(PDFDocument(data: result.data))
        XCTAssertTrue(document.string?.contains("后台原生转换中文") == true)
    }

    func testDOCXDeclaredExpandedSizeIsCheckedBeforeNativeDecode() throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("bomb.docx")
        let source = NSAttributedString(string: "small actual text")
        var data = try source.data(from: NSRange(location: 0, length: source.length), documentAttributes: [.documentType: NSAttributedString.DocumentType.officeOpenXML])
        let central = try XCTUnwrap(data.range(of: Data([0x50, 0x4b, 0x01, 0x02])))
        // Central directory's uncompressed length claims 32 MiB, despite a tiny source.
        data.replaceSubrange((central.lowerBound + 24)..<(central.lowerBound + 28), with: [0x00, 0x00, 0x00, 0x02])
        try data.write(to: input)
        XCTAssertThrowsError(try TextPDFImport.convert(input, to: directory.appendingPathComponent("result.pdf"))) {
            XCTAssertEqual($0 as? ImportFailure, .tooLarge)
        }
    }

    func testDOCXNativeImportWorksOnBackgroundWorker() async throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("background.docx")
        let source = NSAttributedString(string: "后台 Office 中文转换")
        try source.data(from: NSRange(location: 0, length: source.length), documentAttributes: [.documentType: NSAttributedString.DocumentType.officeOpenXML]).write(to: input)
        let result = try await LocalToolJob.run(data: Data(), cancellation: LocalToolCancellation()) { _, output, flag in
            XCTAssertFalse(Thread.isMainThread)
            return "\(try TextPDFImport.convert(input, to: output, cancelled: { flag.isCancelled }))"
        }
        XCTAssertEqual(result.message, "1")
        let document = try XCTUnwrap(PDFDocument(data: result.data))
        XCTAssertTrue(document.string?.contains("后台 Office 中文转换") == true)
    }

    func testDOCXEmbeddedMediaIsRejectedBeforeSystemCanDropIt() throws {
        let directory = try temporaryDirectory()
        let input = directory.appendingPathComponent("media.docx")
        let source = NSAttributedString(string: "native text")
        var data = try source.data(from: NSRange(location: 0, length: source.length), documentAttributes: [.documentType: NSAttributedString.DocumentType.officeOpenXML])
        let original = Data("word/document.xml".utf8)
        let replacement = Data("word/media/a_.xml".utf8)
        XCTAssertEqual(original.count, replacement.count)
        var replacements = 0
        while let range = data.range(of: original) { data.replaceSubrange(range, with: replacement); replacements += 1 }
        XCTAssertGreaterThanOrEqual(replacements, 2)
        try data.write(to: input)
        XCTAssertThrowsError(try TextPDFImport.convert(input, to: directory.appendingPathComponent("result.pdf"))) {
            XCTAssertEqual($0 as? ImportFailure, .invalidInput)
        }
    }
}
