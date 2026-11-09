import XCTest
import AppKit
@testable import PaperDesk

final class SignatureAssetStoreTests: XCTestCase {
    @MainActor func testPersistenceDedupRemovalAndModes() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let source = root.appendingPathComponent("source.png")
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 3, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        try rep.representation(using: .png, properties: [:])!.write(to: source)
        let directory = root.appendingPathComponent("library")
        let store = SignatureAssetStore(directory: directory)
        let asset = try store.importPNG(from: source)
        XCTAssertEqual(try store.importPNG(from: source).id, asset.id)
        let reopened = SignatureAssetStore(directory: directory)
        try reopened.reload()
        XCTAssertEqual(reopened.entries, [asset])
        XCTAssertEqual(try reopened.image(for: asset.id).pixelWidth, 3)
        XCTAssertEqual((try FileManager.default.attributesOfItem(atPath: directory.path)[.posixPermissions] as? NSNumber)?.intValue, 0o700)
        for name in ["index.json", "\(asset.id.uuidString).png"] {
            XCTAssertEqual((try FileManager.default.attributesOfItem(atPath: directory.appendingPathComponent(name).path)[.posixPermissions] as? NSNumber)?.intValue, 0o600)
        }
        try reopened.remove(id: asset.id)
        XCTAssertTrue(FileManager.default.fileExists(atPath: source.path))
        try store.reload()
        XCTAssertTrue(store.entries.isEmpty)
    }

    @MainActor func testRejectsSymlinkAndCorruptIndexWithoutOverwriting() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let source = root.appendingPathComponent("source")
        try Data("invalid".utf8).write(to: source)
        let link = root.appendingPathComponent("link")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: source)
        let directory = root.appendingPathComponent("library")
        let store = SignatureAssetStore(directory: directory)
        XCTAssertThrowsError(try store.importPNG(from: link))
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let index = directory.appendingPathComponent("index.json")
        let bad = Data("broken".utf8)
        try bad.write(to: index)
        XCTAssertThrowsError(try store.importPNG(from: source))
        XCTAssertEqual(try Data(contentsOf: index), bad)
    }

    @MainActor func testStaleInstancesReloadBeforeWritingAndIndexLimitRefusesWrites() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let directory = root.appendingPathComponent("library")
        let first = SignatureAssetStore(directory: directory)
        let second = SignatureAssetStore(directory: directory)
        try first.reload(); try second.reload()
        func source(_ color: NSColor, _ name: String) throws -> URL {
            let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 3, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
            for y in 0..<2 { for x in 0..<3 { rep.setColor(color, atX: x, y: y) } }
            let url = root.appendingPathComponent(name)
            try rep.representation(using: .png, properties: [:])!.write(to: url)
            return url
        }
        let one = try source(NSColor(deviceRed: 1, green: 0, blue: 0, alpha: 1), "one.png")
        let two = try source(NSColor(deviceRed: 0, green: 0, blue: 1, alpha: 1), "two.png")
        _ = try first.importPNG(from: one)
        _ = try second.importPNG(from: two)
        try first.reload()
        XCTAssertEqual(first.entries.count, 2)
        let invalid = (0..<101).map { _ in SignatureAsset(id: UUID(), createdAt: Date(), width: 1, height: 1) }
        let data = try JSONEncoder().encode(invalid)
        try data.write(to: directory.appendingPathComponent("index.json"))
        XCTAssertThrowsError(try first.importPNG(from: one))
        XCTAssertEqual(try Data(contentsOf: directory.appendingPathComponent("index.json")), data)
    }
}
