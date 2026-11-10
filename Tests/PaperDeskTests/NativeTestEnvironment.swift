import Foundation
import XCTest

/// Explicit test-only opt-in. Production never detects XCTest or environment
/// flags to change presentation. Hidden native fixtures remain ordinary tests.
enum NativeTestEnvironment {
    static func allowsForeground(environment: [String: String] = ProcessInfo.processInfo.environment) -> Bool {
        environment["PDFMASTER_TEST_ALLOW_FOREGROUND"] == "1"
    }
    static func requireForeground(file: StaticString = #filePath, line: UInt = #line) throws {
        guard allowsForeground() else {
            throw XCTSkip("Foreground/native presentation excluded from background tests; explicitly set PDFMASTER_TEST_ALLOW_FOREGROUND=1 in a dedicated desktop session.", file: file, line: line)
        }
    }
}
