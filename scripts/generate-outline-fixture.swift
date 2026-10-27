import AppKit
import PDFKit

// Synthetic, non-private fixture for native outline layout and navigation QA.
let output = CommandLine.arguments.dropFirst().first ?? "/tmp/PDFMaster-Outline-QA.pdf"
let document = PDFDocument()
for number in 1...4 {
    let image = NSImage(size: NSSize(width: 600, height: 800))
    image.lockFocus()
    NSColor.white.setFill(); NSRect(x: 0, y: 0, width: 600, height: 800).fill()
    ("Outline QA — Page \(number)" as NSString).draw(at: NSPoint(x: 50, y: 680),
        withAttributes: [.font: NSFont.systemFont(ofSize: 26), .foregroundColor: NSColor.black])
    image.unlockFocus()
    document.insert(PDFPage(image: image)!, at: document.pageCount)
}
let root = PDFOutline()
for number in 1...4 {
    let chapter = PDFOutline(); chapter.label = "Chapter \(number)"
    chapter.destination = PDFDestination(page: document.page(at: number - 1)!, at: .zero)
    for topic in 1...4 {
        let child = PDFOutline()
        child.label = topic == 2 ? " \n\t " : "Topic \(number).\(topic)"
        child.destination = PDFDestination(page: document.page(at: number - 1)!, at: .zero)
        chapter.insertChild(child, at: chapter.numberOfChildren)
    }
    root.insertChild(chapter, at: root.numberOfChildren)
}
document.outlineRoot = root
guard document.write(to: URL(fileURLWithPath: output)) else { fatalError("Cannot write synthetic fixture") }
print(output)
