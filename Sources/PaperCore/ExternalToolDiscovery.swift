import Foundation

public enum ExternalToolDiscovery {
    public static func qpdf(environment: [String: String] = ProcessInfo.processInfo.environment) -> URL? {
        func executable(_ path: String) -> URL? {
            guard path.hasPrefix("/"), FileManager.default.isExecutableFile(atPath: path) else { return nil }
            let url = URL(fileURLWithPath: path).standardizedFileURL
            let attributes = try? FileManager.default.attributesOfItem(atPath: url.resolvingSymlinksInPath().path)
            guard attributes?[.type] as? FileAttributeType == .typeRegular else { return nil }
            return url
        }
        if let explicit = environment["PDFMASTER_QPDF"] { return executable(explicit) }
        for directory in (environment["PATH"] ?? "").split(separator: ":") where directory.hasPrefix("/") {
            if let candidate = executable(String(directory) + "/qpdf") { return candidate }
        }
        return nil
    }
}
