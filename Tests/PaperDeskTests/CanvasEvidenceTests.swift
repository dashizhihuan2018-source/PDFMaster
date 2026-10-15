import AppKit
import PDFKit
import PaperCore
import XCTest
@testable import PaperDesk

/// These are synthetic offscreen fixtures, never screenshots of the user's PDF
/// or a claim of normal desktop interaction while the Mac is locked.
@MainActor final class CanvasEvidenceTests: XCTestCase {
    private func illustration() throws -> NSImage {
        let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 300, pixelsHigh: 220, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        let context = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: bitmap))
        NSGraphicsContext.saveGraphicsState(); NSGraphicsContext.current = context
        NSColor(calibratedRed: 0.91, green: 0.94, blue: 0.99, alpha: 1).setFill(); NSRect(x: 0, y: 0, width: 300, height: 220).fill()
        NSColor(calibratedRed: 0.2, green: 0.4, blue: 0.8, alpha: 1).setFill()
        for index in 0..<3 { NSBezierPath(roundedRect: CGRect(x: 38 + index * 72, y: 35, width: 56, height: 80 + index * 22), xRadius: 6, yRadius: 6).fill() }
        ("PROJECT BOARD" as NSString).draw(at: CGPoint(x: 28, y: 185), withAttributes: [.font: NSFont.systemFont(ofSize: 20, weight: .bold), .foregroundColor: NSColor.darkGray])
        NSGraphicsContext.restoreGraphicsState()
        let image = NSImage(size: NSSize(width: 300, height: 220)); image.addRepresentation(bitmap); return image
    }
    func testSyntheticBoardExportReopenAndLiveInkPreviewEvidence() throws {
        let image = try illustration()
        let bitmap = try XCTUnwrap(image.representations.first as? NSBitmapImageRep)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        let pdf = PDFDocument(); pdf.insert(try XCTUnwrap(PDFPage(image: image)), at: 0)
        let pdfData = try XCTUnwrap(pdf.dataRepresentation())
        let board = CanvasBoard(title: "PDF Master · 项目工作台", elements: [
            CanvasElement(kind: .text, frame: CGRect(x: 48, y: 35, width: 1000, height: 58), text: "PDF Master  /  无限画布", strokeHex: "#20304A", fillHex: "#FFFFFF00"),
            CanvasElement(kind: .sticky, frame: CGRect(x: 50, y: 140, width: 235, height: 205), text: "项目目标\n\n把 PDF、草图和想法\n整理到同一块画布", fillHex: "#FFF0A6"),
            CanvasElement(kind: .sticky, frame: CGRect(x: 345, y: 140, width: 230, height: 205), text: "审阅清单\n\n☑ 阅读资料\n☑ 形状标记\n☑ 本地保存", fillHex: "#CDEADD"),
            CanvasElement(kind: .rectangle, frame: CGRect(x: 640, y: 145, width: 210, height: 90), text: "方案与批注", strokeHex: "#397DEA", fillHex: "#DDE9FF", lineWidth: 3),
            CanvasElement(kind: .ellipse, frame: CGRect(x: 930, y: 140, width: 200, height: 95), text: "待讨论", strokeHex: "#9476C5", fillHex: "#EDE4FF", lineWidth: 3),
            CanvasElement(kind: .connector, frame: CGRect(x: 860, y: 183, width: 60, height: 1), strokeHex: "#647493", fillHex: "#FFFFFF00", lineWidth: 3, points: [CGPoint(x: 860, y: 183), CGPoint(x: 920, y: 183)]),
            CanvasElement(kind: .image, frame: CGRect(x: 50, y: 410, width: 300, height: 220), assetData: png, assetName: "合成概念图.png"),
            CanvasElement(kind: .pdf, frame: CGRect(x: 415, y: 400, width: 260, height: 240), assetData: pdfData, assetName: "自建演示.pdf"),
            CanvasElement(kind: .text, frame: CGRect(x: 740, y: 360, width: 390, height: 80), text: "实时手绘\n拖动时即可看到笔迹", strokeHex: "#20304A", fillHex: "#FFFFFF00")
        ])
        let session = CanvasSession(board: try board.validated())
        let view = CanvasView(session: session); view.frame = CGRect(x: 0, y: 0, width: 1200, height: 760)
        let before = try XCTUnwrap(view.renderPNG())
        view.tool = .ink
        view.beginGesture(at: CGPoint(x: 760, y: 510))
        for point in [CGPoint(x: 790, y: 460), CGPoint(x: 830, y: 545), CGPoint(x: 880, y: 470), CGPoint(x: 925, y: 525), CGPoint(x: 990, y: 455), CGPoint(x: 1060, y: 515)] { view.updateGesture(to: point) }
        let during = try XCTUnwrap(view.renderPNG())
        XCTAssertNotEqual(before, during)
        XCTAssertEqual(session.board.elements.count, 9, "拖动途中只预览，不修改已提交模型")
        view.finishGesture()
        XCTAssertEqual(session.board.elements.count, 10)
        let after = try XCTUnwrap(view.renderPNG())
        XCTAssertEqual(try CanvasBoard.decode(session.board.encoded()), session.board)
        if let path = ProcessInfo.processInfo.environment["PDFMASTER_CANVAS_CAPTURE_DIR"] {
            let dir = URL(fileURLWithPath: path, isDirectory: true)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            try before.write(to: dir.appendingPathComponent("canvas-before.png"), options: .atomic)
            try during.write(to: dir.appendingPathComponent("canvas-live-preview.png"), options: .atomic)
            try after.write(to: dir.appendingPathComponent("canvas-completed.png"), options: .atomic)
            try session.board.save(to: dir.appendingPathComponent("PDF Master 无限画布演示.pdfmasterboard"))
        }
    }
}
