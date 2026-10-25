import Foundation
import XCTest
import Darwin

final class NativeStandalonePackagingTests: XCTestCase {
    private var root: URL { URL(fileURLWithPath:#filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent() }

    func testPackagedResourcesHaveNoPythonAndBundledQPDFRunsAfterRelocation() throws {
        let staged = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-ResourceStage-\(UUID())")
        let relocated = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-Resources-\(UUID())")
        defer { try? FileManager.default.removeItem(at:staged); try? FileManager.default.removeItem(at:relocated) }
        let resources = staged.appendingPathComponent("Contents/Resources")
        try FileManager.default.createDirectory(at:resources,withIntermediateDirectories:true)
        let environment = ProcessInfo.processInfo.environment
        let binary = try XCTUnwrap(environment["PDFMASTER_TEST_QPDF"], "Set PDFMASTER_TEST_QPDF to the portable qpdf binary")
        let library = try XCTUnwrap(environment["PDFMASTER_TEST_QPDF_LIB"], "Set PDFMASTER_TEST_QPDF_LIB to its dylib")
        let qpdfSource = try XCTUnwrap(environment["PDFMASTER_TEST_QPDF_SOURCE"], "Set official qpdf license source directory")
        let jpegSource = try XCTUnwrap(environment["PDFMASTER_TEST_JPEG_SOURCE"], "Set official libjpeg-turbo license source directory")
        let packaged = try run(URL(fileURLWithPath:"/bin/bash"),[root.appendingPathComponent("scripts/package-local-resources.sh").path,staged.path,"--qpdf",binary,"--qpdf-lib",library,"--qpdf-source",qpdfSource,"--jpeg-source",jpegSource])
        XCTAssertEqual(packaged.0,0,packaged.1)
        let enumerator = try XCTUnwrap(FileManager.default.enumerator(at:resources,includingPropertiesForKeys:[.isRegularFileKey]))
        let files = enumerator.allObjects.compactMap { $0 as? URL }
        let python = files.filter { ["py","pyc","pyo"].contains($0.pathExtension.lowercased()) }
        XCTAssertTrue(python.isEmpty,"Distribution must not contain old engine scripts: \(python.map(\.lastPathComponent))")
        XCTAssertFalse(FileManager.default.fileExists(atPath:resources.appendingPathComponent("Engines/content-edit").path))
        XCTAssertTrue(FileManager.default.fileExists(atPath:resources.appendingPathComponent("CanvasWeb/index.html").path))
        XCTAssertTrue(FileManager.default.fileExists(atPath:resources.appendingPathComponent("Licenses/qpdf/LICENSE.txt").path))
        try FileManager.default.copyItem(at:resources,to:relocated)
        let qpdf = relocated.appendingPathComponent("Engines/qpdf/bin/qpdf")
        let version = try run(qpdf,["--version"],environment:["PATH":"/does-not-exist","LANG":"en_US.UTF-8"])
        XCTAssertEqual(version.0,0,version.1); XCTAssertTrue(version.1.contains("qpdf version 12.4.2"))
        for binary in [qpdf,relocated.appendingPathComponent("Engines/qpdf/lib/libqpdf.30.dylib")] {
            let signature = try run(URL(fileURLWithPath:"/usr/bin/codesign"),["--verify","--strict",binary.path])
            XCTAssertEqual(signature.0,0,signature.1)
            let dependencies = try run(URL(fileURLWithPath:"/usr/bin/otool"),["-L",binary.path])
            XCTAssertEqual(dependencies.0,0,dependencies.1)
            for line in dependencies.1.split(separator:"\n").dropFirst() {
                let dependency = line.trimmingCharacters(in:.whitespaces).split(separator:" ").first.map(String.init) ?? ""
                XCTAssertTrue(dependency.hasPrefix("@rpath/") || dependency.hasPrefix("/usr/lib/") || dependency.hasPrefix("/System/"),"External dependency: \(dependency)")
            }
        }
    }

    func testReleaseContentResolverDoesNotFallBackToDeveloperCheckout() throws {
        let source = try String(contentsOf:root.appendingPathComponent("Sources/PaperCore/ContentEditEngine.swift"),encoding:.utf8)
        let start = try XCTUnwrap(source.range(of:"private var qpdf:")), end = try XCTUnwrap(source.range(of:"public func validateAdmission"))
        let resolver = String(source[start.lowerBound..<end.lowerBound])
        let release = resolver.replacingOccurrences(of:"#if DEBUG[\\s\\S]*?#endif",with:"",options:.regularExpression)
        XCTAssertFalse(release.contains("#filePath")); XCTAssertFalse(release.contains("tools/qpdf-"))
        XCTAssertTrue(release.contains("Engines/qpdf/bin/qpdf"))
    }

    func testNativeFontRunnerNeverSpinsAFoundationWaitRunLoop() throws {
        let source = try String(contentsOf:root.appendingPathComponent("Sources/PaperCore/NativeFontRepair.swift"),encoding:.utf8)
        XCTAssertFalse(source.contains(".waitUntilExit()"),"Process.waitUntilExit may deliver PDFKit UI notifications on a worker run loop")
    }

    func testActualReleaseObjectUsesOnlyBundledQPDFWithEmptyEnvironment() throws {
        let environment = ProcessInfo.processInfo.environment
        guard let productPath = environment["PDFMASTER_TEST_RELEASE_PRODUCTS"] else {
            throw XCTSkip("Build release first and set PDFMASTER_TEST_RELEASE_PRODUCTS to its products directory")
        }
        let products = URL(fileURLWithPath: productPath)
        XCTAssertTrue(FileManager.default.fileExists(atPath: products.appendingPathComponent("PaperCore.o").path))
        let binary = URL(fileURLWithPath: try XCTUnwrap(environment["PDFMASTER_TEST_QPDF"]))
        let library = URL(fileURLWithPath: try XCTUnwrap(environment["PDFMASTER_TEST_QPDF_LIB"]))
        let fixture = try PublicContentFixture(pageCount: 5, title: "示例古建", imagePageCount: 4)
        let bundle = fixture.directory.appendingPathComponent("ReleaseProbe")
        let executable = bundle.appendingPathComponent("Contents/MacOS/ReleaseProbe")
        let resources = bundle.appendingPathComponent("Contents/Resources")
        try FileManager.default.createDirectory(at: executable.deletingLastPathComponent(), withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: resources, withIntermediateDirectories: true)
        let source = fixture.directory.appendingPathComponent("probe.swift")
        let program = #"""
        import Foundation
        import PDFKit
        import PaperCore
        @main struct ReleaseProbe {
            static func main() async {
                do {
                    let source = URL(fileURLWithPath: CommandLine.arguments[1])
                    let original = try Data(contentsOf: source)
                    let engine = ContentEditEngine()
                    let first = try engine.inspect(source, page: 0, cancelled: { false })
                    guard first.images.count == 1 else { throw OrganizeError.invalidInput }
                    let workspace = PDFWorkspace(); try workspace.open(source)
                    let input = try workspace.engineInput()
                    let result = try await LocalToolJob.run(data: input.data, cancellation: LocalToolCancellation()) { source, output, flag in
                        for raw in input.safetySourceDatas {
                            try engine.validateAdmission(raw, in: source.deletingLastPathComponent(), cancelled: { flag.isCancelled })
                        }
                        let page = try engine.inspect(source, page: 0, cancelled: { flag.isCancelled })
                        guard let target = page.spans.first(where: { $0.text == "示例古建" }) else { throw OrganizeError.invalidInput }
                        var edit = ContentEditRequest(operation: "replace-text", page: 0)
                        edit.id = target.id; edit.text = "阅读示例"
                        return try engine.apply(source, request: edit, output: output, cancelled: { flag.isCancelled })
                    }
                    try workspace.acceptEngineResult(result.data, for: input)
                    guard let page = workspace.document?.page(at: 0), workspace.document?.pageCount == 5,
                          page.string?.contains("阅读示例") == true, page.string?.contains("示例古建") == false,
                          try Data(contentsOf: source) == original else { throw OrganizeError.invalidInput }
                    print("RELEASE_BUNDLE_PASS")
                } catch { print("RELEASE_BUNDLE_ERROR", String(describing: error)); exit(1) }
            }
        }
        """#
        try Data(program.utf8).write(to: source)
        let compiled = try run(URL(fileURLWithPath: "/usr/bin/xcrun"), ["swiftc", "-parse-as-library", "-target", "arm64-apple-macos14.0", "-I", products.path, source.path, products.appendingPathComponent("PaperCore.o").path, "-o", executable.path], environment: environment)
        XCTAssertEqual(compiled.0, 0, compiled.1)
        // A release object must ignore even valid explicit developer overrides.
        let absent = try run(executable, [fixture.source.path], environment: ["PATH": "/does-not-exist", "PDFMASTER_TEST_QPDF": binary.path, "PDFMASTER_QPDF": binary.path])
        XCTAssertEqual(absent.0, 1, absent.1)
        XCTAssertTrue(absent.1.contains("RELEASE_BUNDLE_ERROR unavailable"), absent.1)
        let engine = resources.appendingPathComponent("Engines/qpdf")
        try FileManager.default.createDirectory(at: engine.appendingPathComponent("bin"), withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: engine.appendingPathComponent("lib"), withIntermediateDirectories: true)
        try FileManager.default.copyItem(at: binary, to: engine.appendingPathComponent("bin/qpdf"))
        try FileManager.default.copyItem(at: library, to: engine.appendingPathComponent("lib/libqpdf.30.dylib"))
        let present = try run(executable, [fixture.source.path], environment: [:])
        XCTAssertEqual(present.0, 0, present.1)
        XCTAssertTrue(present.1.contains("RELEASE_BUNDLE_PASS"), present.1)
    }

    private func run(_ executable:URL,_ arguments:[String],environment:[String:String] = ["PATH":"/usr/bin:/bin:/usr/sbin:/sbin","LANG":"en_US.UTF-8"]) throws -> (Int32,String) {
        let log = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-PackagingLog-\(UUID())")
        guard FileManager.default.createFile(atPath:log.path,contents:nil,attributes:[.posixPermissions:0o600]) else { throw NSError(domain:"PackagingTest",code:1) }
        defer { try? FileManager.default.removeItem(at:log) }
        let file = try FileHandle(forWritingTo:log); defer { try? file.close() }
        let process = Process(); process.executableURL = executable; process.arguments = arguments
        process.standardOutput = file; process.standardError = file; process.standardInput = FileHandle.nullDevice; process.environment = environment
        try process.run(); let deadline = Date().addingTimeInterval(45)
        while process.isRunning && Date() < deadline { Thread.sleep(forTimeInterval:0.01) }
        if process.isRunning { process.terminate(); let stop = Date().addingTimeInterval(0.5); while process.isRunning && Date() < stop { Thread.sleep(forTimeInterval:0.01) }; if process.isRunning { kill(process.processIdentifier,SIGKILL) }; throw NSError(domain:"PackagingTimeout",code:2) }
        try file.synchronize()
        let bytes = try Data(contentsOf:log)
        return (process.terminationStatus,String(decoding:bytes.prefix(100000),as:UTF8.self))
    }
}
