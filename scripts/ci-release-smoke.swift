import AppKit
import CoreText
import PDFKit

func require(_ condition: @autoclosure () -> Bool, _ message: String) {
    if !condition() { fputs("FAIL: \(message)\n", stderr); exit(1) }
}
func spin(until ready: () -> Bool, seconds: TimeInterval) -> Bool {
    let deadline = Date().addingTimeInterval(seconds)
    while !ready(), Date() < deadline {
        RunLoop.current.run(until: Date().addingTimeInterval(0.1))
    }
    return ready()
}

let args = CommandLine.arguments
require(args.count == 3, "Usage: probe APP_OR_--fixture-only EXISTING_OUTPUT_DIRECTORY")
let directory = URL(fileURLWithPath: args[2], isDirectory: true)
let sample = directory.appendingPathComponent("public-smoke.pdf")
require(!FileManager.default.fileExists(atPath: sample.path), "Do not overwrite a sample")
var page = CGRect(x: 0, y: 0, width: 420, height: 594)
guard let context = CGContext(sample as CFURL, mediaBox: &page, nil) else { fatalError("Cannot create synthetic PDF") }
context.beginPDFPage(nil)
context.textPosition = CGPoint(x: 40, y: 500)
let text = NSAttributedString(string: "PDF Master free distribution smoke test", attributes: [.font: NSFont.systemFont(ofSize: 14)])
CTLineDraw(CTLineCreateWithAttributedString(text), context)
context.endPDFPage()
context.closePDF()
guard let document = PDFDocument(url: sample) else { fatalError("Cannot read synthetic PDF") }
require(document.pageCount == 1 && document.string?.contains("distribution smoke test") == true, "Synthetic document text and page count")
if args[1] == "--fixture-only" { print("PASS: self-authored fixture generated and read with PDFKit"); exit(0) }

require(ProcessInfo.processInfo.environment["GITHUB_ACTIONS"] == "true", "Launch probe is restricted to a disposable CI VM")
let appURL = URL(fileURLWithPath: args[1], isDirectory: true).resolvingSymlinksInPath()
let qpdf = Process()
qpdf.executableURL = appURL.appendingPathComponent("Contents/Resources/Engines/qpdf/bin/qpdf")
qpdf.arguments = ["--check", sample.path]
qpdf.environment = ["PATH": "/usr/bin:/bin"]
try qpdf.run()
qpdf.waitUntilExit()
require(qpdf.terminationStatus == 0, "Bundled qpdf must process the self-authored PDF")

let configuration = NSWorkspace.OpenConfiguration()
configuration.createsNewApplicationInstance = true
configuration.activates = false
configuration.environment = ["PATH": "/usr/bin:/bin"]
var application: NSRunningApplication?
var failure: Error?
var resolved = false
NSWorkspace.shared.open([sample], withApplicationAt: appURL, configuration: configuration) { app, error in
    application = app
    failure = error
    resolved = true
}
require(spin(until: { resolved }, seconds: 30), "Launch Services callback timeout")
if let failure { fputs("Launch Services error: \(failure.localizedDescription)\n", stderr) }
guard let app = application else { fatalError("Launch Services did not start the public App") }
require(app.bundleURL?.resolvingSymlinksInPath() == appURL, "Actual launched bundle must be the downloaded copy")
let healthyUntil = Date().addingTimeInterval(10)
while Date() < healthyUntil {
    require(!app.isTerminated, "App exited or crashed during startup")
    RunLoop.current.run(until: Date().addingTimeInterval(0.2))
}
print("PASS: downloaded App remained running for 10 seconds after document-open request")
require(app.terminate(), "App must accept a normal quit request")
require(spin(until: { app.isTerminated }, seconds: 20), "App did not quit normally")
print("PASS: App quit normally; no force termination or security settings changed")
