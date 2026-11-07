import Foundation

// Self-authored, no private PDF or external assets. For native acceptance only.
guard CommandLine.arguments.count == 2 else { fatalError("Pass a new output PDF path") }
let output = URL(fileURLWithPath: CommandLine.arguments[1])
guard !FileManager.default.fileExists(atPath: output.path) else { fatalError("Refusing overwrite") }
func stream(_ page: Int) -> String {
    let content = """
    BT /F1 22 Tf 40 700 Td (PUBLIC SAMPLE - PAGE \(page)) Tj ET
    BT /F1 14 Tf 40 650 Td (Body text and ordinary note must remain.) Tj ET
    /Artifact << /Type /Pagination /Subtype /Watermark >> BDC
    q 0.72 g BT /F1 32 Tf 55 400 Td (SAMPLE WATERMARK) Tj ET Q
    EMC

    """
    return "<< /Length \(content.utf8.count) >>\nstream\n\(content)endstream"
}
let objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 2 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /Annots [8 0 R 9 0 R] >>",
    stream(1),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 7 0 R >>",
    stream(2),
    "<< /Type /Annot /Subtype /Watermark /Rect [30 30 100 60] /Contents (Explicit Watermark Annotation) >>",
    "<< /Type /Annot /Subtype /Text /Rect [40 580 65 605] /Contents (Ordinary note - keep me) >>"
]
var bytes = Data("%PDF-1.7\n".utf8), offsets = [0]
for (index, object) in objects.enumerated() {
    offsets.append(bytes.count)
    bytes.append(Data("\(index + 1) 0 obj\n\(object)\nendobj\n".utf8))
}
let xref = bytes.count
bytes.append(Data("xref\n0 \(objects.count + 1)\n0000000000 65535 f \n".utf8))
for offset in offsets.dropFirst() { bytes.append(Data(String(format: "%010d 00000 n \n", offset).utf8)) }
bytes.append(Data("trailer\n<< /Size \(objects.count + 1) /Root 1 0 R >>\nstartxref\n\(xref)\n%%EOF\n".utf8))
try bytes.write(to: output, options: .withoutOverwriting)
print("Created self-authored two-page watermark fixture")
