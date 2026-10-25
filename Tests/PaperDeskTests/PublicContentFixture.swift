import Foundation
import AppKit

/// Only self-authored text, a four-pixel color pattern, and basic vector geometry.
/// No downloaded document, font program, Python interpreter, or user data is used.
final class PublicContentFixture {
    let directory: URL
    let source: URL
    let replacement: URL
    init(pageCount: Int = 20, title: String = "中文原文", imagePageCount: Int = 1, titleSize: Int = 14, titleFont: String = "STSong-Light") throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-PublicFixture-" + UUID().uuidString)
        source = directory.appendingPathComponent("source.pdf")
        replacement = directory.appendingPathComponent("replacement.png")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        try Self.pdf(pageCount: pageCount, title: title, imagePageCount: imagePageCount, titleSize: titleSize, titleFont: titleFont).write(to: source)
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 4, pixelsHigh: 4, bitsPerSample: 8, samplesPerPixel: 3, hasAlpha: false, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 12, bitsPerPixel: 24)!
        for y in 0..<4 { for x in 0..<4 { let pixel = bitmap.bitmapData!.advanced(by: y * bitmap.bytesPerRow + x * 3); pixel[0] = UInt8(x * 85); pixel[1] = UInt8(y * 85); pixel[2] = 179 } }
        try bitmap.representation(using: .png, properties: [:])!.write(to: replacement)
    }
    deinit { try? FileManager.default.removeItem(at: directory) }

    static func pdf(pageCount: Int = 20, title: String = "中文原文", imagePageCount: Int = 1, titleSize: Int = 14, titleFont: String = "STSong-Light") -> Data {
        var objects = ["<< /Type /Catalog /Pages 2 0 R >>", "",
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            "<< /Type /Font /Subtype /Type0 /BaseFont /\(titleFont) /Encoding /UniGB-UTF16-H /DescendantFonts [5 0 R] >>",
            "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /\(titleFont) /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 5 >> /FontDescriptor 6 0 R /DW 1000 >>",
            "<< /Type /FontDescriptor /FontName /\(titleFont) /FontBBox [0 -120 1000 880] /Flags 4 /ItalicAngle 0 /Ascent 880 /Descent -120 /StemV 80 >>"]
        var pages: [Int] = []
        for page in 0..<pageCount {
            let pageID = objects.count + 1, streamID = pageID + 1, imageID = pageID + 2
            pages.append(pageID)
            let hex = title.utf16.map { String(format: "%04X", $0) }.joined()
            var stream = "BT /F1 14 Tf 1 0 0 1 40 500 Tm (Original target) Tj ET\nBT /F1 14 Tf 1 0 0 1 40 460 Tm (Neighbor preserved) Tj ET\nBT /CJK \(titleSize) Tf 1 0 0 1 40 180 Tm <\(hex)> Tj ET\n0.2 0.4 0.6 rg 260 300 60 40 re f\n"
            let hasImage = page < imagePageCount
            if hasImage { stream += "q 80 0 0 60 40 300 cm /Im0 Do Q\n" }
            objects.append("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 420 594] /Resources << /Font << /F1 3 0 R /CJK 4 0 R >> \(hasImage ? "/XObject << /Im0 \(imageID) 0 R >>" : "") >> /Contents \(streamID) 0 R >>")
            objects.append("<< /Length \(stream.utf8.count) >>\nstream\n\(stream)endstream")
            if hasImage {
                let pixels = String(format: "%02X", 20 + page * 10) + "000000FF000000FFFFFF00>"
                objects.append("<< /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /ASCIIHexDecode /Length \(pixels.utf8.count) >>\nstream\n\(pixels)\nendstream")
            }
        }
        objects[1] = "<< /Type /Pages /Kids [\(pages.map { "\($0) 0 R" }.joined(separator: " "))] /Count \(pageCount) >>"
        var pdf = "%PDF-1.7\n", offsets = [0]
        for (index, object) in objects.enumerated() { offsets.append(pdf.utf8.count); pdf += "\(index + 1) 0 obj\n\(object)\nendobj\n" }
        let xref = pdf.utf8.count
        pdf += "xref\n0 \(objects.count + 1)\n0000000000 65535 f \n" + offsets.dropFirst().map { String(format: "%010d 00000 n \n", $0) }.joined()
        pdf += "trailer\n<< /Size \(objects.count + 1) /Root 1 0 R >>\nstartxref\n\(xref)\n%%EOF\n"
        return Data(pdf.utf8)
    }
}
