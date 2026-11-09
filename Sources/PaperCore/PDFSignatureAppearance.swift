import AppKit
import ImageIO
import PDFKit
import ObjectiveC

private var signatureBackingKey: UInt8 = 0

/// Builds a standard PDF appearance, including a soft mask, with no file references.
enum PDFSignatureAppearance {
    static func make(_ image: SignatureImage, bounds: CGRect, rotation: Int) throws -> PDFAnnotation {
        guard let source = CGImageSourceCreateWithData(image.pngData as CFData, nil),
              let cg = CGImageSourceCreateImageAtIndex(source, 0, nil) else { throw PDFWorkspaceError.invalidSelection }
        let w = image.pixelWidth, h = image.pixelHeight
        var rgba = [UInt8](repeating: 0, count: w * h * 4)
        let decoded = rgba.withUnsafeMutableBytes { bytes -> Bool in
            guard let context = CGContext(data: bytes.baseAddress, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
                                          space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else { return false }
            context.draw(cg, in: CGRect(x: 0, y: 0, width: w, height: h))
            return true
        }
        guard decoded else { throw PDFWorkspaceError.invalidSelection }
        var rgb = Data(capacity: w * h * 3), alpha = Data(capacity: w * h)
        for i in 0..<(w * h) {
            let a = Int(rgba[4*i+3]); alpha.append(UInt8(a))
            for c in 0..<3 { rgb.append(a == 0 ? 0 : UInt8(min(255, (Int(rgba[4*i+c]) * 255 + a/2) / a))) }
        }
        let transform: String
        switch ((rotation % 360) + 360) % 360 {
        case 90: transform = "0 1 -1 0 1 0"
        case 180: transform = "-1 0 0 -1 1 1"
        case 270: transform = "0 -1 1 0 0 1"
        default: transform = "1 0 0 1 0 0"
        }
        let commands = "q \(transform) cm /Im0 Do Q\n"
        func stream(_ dictionary: String, _ bytes: Data) -> Data {
            var result = Data("<< \(dictionary) /Length \(bytes.count) >>\nstream\n".utf8)
            result.append(bytes); result.append(Data("\nendstream".utf8)); return result
        }
        let objects: [Data] = [
            Data("<< /Type /Catalog /Pages 2 0 R >>".utf8),
            Data("<< /Type /Pages /Kids [3 0 R] /Count 1 >>".utf8),
            Data("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /Annots [4 0 R] >>".utf8),
            Data("<< /Type /Annot /Subtype /Stamp /Rect [0 0 1 1] /F 4 /AP << /N 5 0 R >> >>".utf8),
            stream("/Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Im0 6 0 R >> >>", Data(commands.utf8)),
            stream("/Type /XObject /Subtype /Image /Width \(w) /Height \(h) /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask 7 0 R", rgb),
            stream("/Type /XObject /Subtype /Image /Width \(w) /Height \(h) /ColorSpace /DeviceGray /BitsPerComponent 8", alpha)
        ]
        var pdf = Data("%PDF-1.7\n".utf8), offsets = [0]
        for (index, object) in objects.enumerated() {
            offsets.append(pdf.count); pdf.append(Data("\(index + 1) 0 obj\n".utf8)); pdf.append(object); pdf.append(Data("\nendobj\n".utf8))
        }
        let xref = pdf.count
        pdf.append(Data("xref\n0 \(objects.count + 1)\n0000000000 65535 f \n".utf8))
        for offset in offsets.dropFirst() { pdf.append(Data(String(format: "%010d 00000 n \n", offset).utf8)) }
        pdf.append(Data("trailer\n<< /Size \(objects.count + 1) /Root 1 0 R >>\nstartxref\n\(xref)\n%%EOF\n".utf8))
        guard let document = PDFDocument(data: pdf), let sourcePage = document.page(at: 0),
              let original = sourcePage.annotations.first, let annotation = original.copy() as? PDFAnnotation else { throw PDFWorkspaceError.writeFailed }
        objc_setAssociatedObject(annotation, &signatureBackingKey, document, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
        annotation.bounds = bounds
        return annotation
    }
}
