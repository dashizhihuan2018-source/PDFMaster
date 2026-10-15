import Foundation
import CoreGraphics
import PDFKit

/// Native, fail-closed admission for portable canvas PDF attachments. No helper
/// process, password removal, active action execution, or PDF normalization.
enum PassivePDFCopyAdmission {
    private final class Entries {
        var values: [(String, CGPDFObjectRef)] = []
    }

    private static let collect: CGPDFDictionaryApplierFunction = { key, value, context in
        guard let context else { return }
        Unmanaged<Entries>.fromOpaque(context).takeUnretainedValue().values.append((String(cString: key), value))
    }

    static func validate(_ data: Data) throws -> PDFDocument {
        guard !data.isEmpty, data.count <= 50 * 1_024 * 1_024,
              let provider = CGDataProvider(data: data as CFData),
              let raw = CGPDFDocument(provider), !raw.isEncrypted,
              (1...500).contains(raw.numberOfPages), let catalog = raw.catalog
        else { throw PDFWorkspaceError.unreadableDocument }

        var seenDictionaries = Set<UInt>(), seenArrays = Set<UInt>(), budget = 200_000
        let forbiddenKeys: Set<String> = ["OpenAction", "AA", "JS", "JavaScript", "AcroForm", "XFA", "EmbeddedFiles", "EF", "RichMediaContent", "RichMediaSettings", "Collection", "ByteRange", "Perms"]
        let activeActions: Set<String> = ["JavaScript", "Launch", "GoToR", "GoToE", "URI", "SubmitForm", "ResetForm", "ImportData", "Rendition", "RichMediaExecute", "Sound", "Movie", "SetOCGState", "Hide", "Named", "Thread", "Trans"]
        let safeAnnotations: Set<String> = ["Text", "Popup", "FreeText", "Line", "Square", "Circle", "Polygon", "PolyLine", "Highlight", "Underline", "Squiggly", "StrikeOut", "Stamp", "Caret", "Ink", "Redact", "Link"]

        func consume(_ depth: Int) throws {
            guard depth <= 64, budget > 0 else { throw PDFWorkspaceError.unreadableDocument }
            budget -= 1
        }
        func name(_ dictionary: CGPDFDictionaryRef, _ key: String) -> String? {
            var pointer: UnsafePointer<CChar>?
            guard CGPDFDictionaryGetName(dictionary, key, &pointer), let pointer else { return nil }
            return String(cString: pointer)
        }
        func dictionary(_ dictionary: CGPDFDictionaryRef, depth: Int) throws {
            guard seenDictionaries.insert(UInt(bitPattern: dictionary.rawValue)).inserted else { return }
            try consume(depth)
            let entries = Entries()
            CGPDFDictionaryApplyFunction(dictionary, collect, Unmanaged.passUnretained(entries).toOpaque())
            guard entries.values.count <= budget else { throw PDFWorkspaceError.unreadableDocument }
            guard !entries.values.contains(where: { forbiddenKeys.contains($0.0) }),
                  name(dictionary, "FT") != "Sig", name(dictionary, "Type") != "Sig",
                  !activeActions.contains(name(dictionary, "S") ?? "")
            else { throw PDFWorkspaceError.unreadableDocument }
            if name(dictionary, "Type") == "Annot" || entries.values.contains(where: { $0.0 == "Rect" && name(dictionary, "Subtype") != nil }) {
                guard let subtype = name(dictionary, "Subtype"), safeAnnotations.contains(subtype) else { throw PDFWorkspaceError.unreadableDocument }
                // Only internal GoTo actions are passive in an annotation. An
                // unknown action is not assumed safe; ordinary text is untouched.
                if entries.values.contains(where: { $0.0 == "A" }) {
                    var action: CGPDFDictionaryRef?
                    guard CGPDFDictionaryGetDictionary(dictionary, "A", &action),
                          let action, name(action, "S") == "GoTo" else { throw PDFWorkspaceError.unreadableDocument }
                }
            }
            for (_, value) in entries.values { try object(value, depth: depth + 1) }
        }
        func array(_ array: CGPDFArrayRef, depth: Int) throws {
            guard seenArrays.insert(UInt(bitPattern: array.rawValue)).inserted else { return }
            try consume(depth)
            let count = CGPDFArrayGetCount(array)
            guard count <= budget else { throw PDFWorkspaceError.unreadableDocument }
            for index in 0..<count {
                var entry: CGPDFObjectRef?
                guard CGPDFArrayGetObject(array, index, &entry), let entry else { throw PDFWorkspaceError.unreadableDocument }
                try object(entry, depth: depth + 1)
            }
        }
        func object(_ object: CGPDFObjectRef, depth: Int) throws {
            switch CGPDFObjectGetType(object) {
            case .dictionary:
                var value: CGPDFDictionaryRef?
                guard CGPDFObjectGetValue(object, .dictionary, &value), let value else { throw PDFWorkspaceError.unreadableDocument }
                try dictionary(value, depth: depth)
            case .array:
                var value: CGPDFArrayRef?
                guard CGPDFObjectGetValue(object, .array, &value), let value else { throw PDFWorkspaceError.unreadableDocument }
                try array(value, depth: depth)
            case .stream:
                var value: CGPDFStreamRef?
                guard CGPDFObjectGetValue(object, .stream, &value), let value,
                      let valueDictionary = CGPDFStreamGetDictionary(value) else { throw PDFWorkspaceError.unreadableDocument }
                try dictionary(valueDictionary, depth: depth)
            default: try consume(depth)
            }
        }
        try dictionary(catalog, depth: 0)
        guard let loaded = PDFDocument(data: data), !loaded.isEncrypted, !loaded.isLocked,
              loaded.pageCount == raw.numberOfPages else { throw PDFWorkspaceError.unreadableDocument }
        var annotationCount = 0
        func finiteBox(_ bounds: CGRect) -> Bool {
            bounds.origin.x.isFinite && bounds.origin.y.isFinite &&
                abs(bounds.origin.x) <= 1_000_000 && abs(bounds.origin.y) <= 1_000_000 &&
                bounds.size.width.isFinite && bounds.size.height.isFinite &&
                bounds.size.width > 0 && bounds.size.height > 0 &&
                bounds.size.width <= 20_000 && bounds.size.height <= 20_000
        }
        for index in 0..<loaded.pageCount {
            guard let page = loaded.page(at: index), finiteBox(page.bounds(for: .mediaBox)),
                  finiteBox(page.bounds(for: .cropBox)) else { throw PDFWorkspaceError.unreadableDocument }
            annotationCount += page.annotations.count
            guard annotationCount <= 20_000 else { throw PDFWorkspaceError.unreadableDocument }
            for annotation in page.annotations {
                guard let type = annotation.type, safeAnnotations.contains(type),
                      finiteBox(annotation.bounds) else { throw PDFWorkspaceError.unreadableDocument }
            }
        }
        return loaded
    }
}
