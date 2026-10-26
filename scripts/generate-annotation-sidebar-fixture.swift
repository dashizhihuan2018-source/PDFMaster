#!/usr/bin/env swift
import AppKit
import PDFKit

guard CommandLine.arguments.count == 2 else { exit(64) }
let output = URL(fileURLWithPath: CommandLine.arguments[1])
guard !FileManager.default.fileExists(atPath: output.path) else { exit(73) }
let document = PDFDocument()
for index in 0..<2 {
    let page = PDFPage()
    page.setBounds(CGRect(x: 0, y: 0, width: 600, height: 800), for: .mediaBox)
    for row in 0..<8 {
        let note = PDFAnnotation(bounds: CGRect(x: 40, y: 700 - row * 70, width: 180, height: 35),
                                 forType: row.isMultiple(of: 2) ? .square : .line, withProperties: nil)
        note.color = .systemRed
        if row == 2 { note.contents = "Review dimensions before approval. A longer comment must stay compact." }
        if row != 7 { note.modificationDate = Date(timeIntervalSince1970: 1_792_981_800 + Double(index * 3600 + row * 60)) }
        if row == 7 { note.modificationDate = nil }
        page.addAnnotation(note)
    }
    document.insert(page, at: index)
}
guard document.write(to: output), PDFDocument(url: output)?.pageCount == 2 else { exit(74) }
print(output.path)
