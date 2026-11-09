import XCTest
import AppKit
import PaperCore
@testable import PaperDesk

final class SignatureAssetStoreTests: XCTestCase {
    @MainActor func testMissingAssetCanBeRemovedAndLibraryReused() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 3, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        let source = root.appendingPathComponent("source.png")
        try rep.representation(using: .png, properties: [:])!.write(to: source)
        let directory = root.appendingPathComponent("library")
        let store = SignatureAssetStore(directory: directory)
        let asset = try store.importPNG(from: source)
        let stored = directory.appendingPathComponent("\(asset.id.uuidString).png")
        try FileManager.default.removeItem(at: stored)
        try FileManager.default.createSymbolicLink(at: stored, withDestinationURL: source)
        XCTAssertThrowsError(try store.remove(id: asset.id))
        XCTAssertTrue(FileManager.default.fileExists(atPath: source.path))
        XCTAssertEqual(store.entries.count, 1)
        try FileManager.default.removeItem(at: stored)
        try store.remove(id: asset.id)
        let reopened = SignatureAssetStore(directory: directory)
        try reopened.reload()
        XCTAssertTrue(reopened.entries.isEmpty)
        XCTAssertNoThrow(try reopened.importPNG(from: source))
    }

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
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 3, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        try rep.representation(using: .png, properties: [:])!.write(to: source)
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

    @MainActor func testLocalURLsAndFaultsAndByteBudget() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 3, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        let source = root.appendingPathComponent("valid.png")
        try rep.representation(using: .png, properties: [:])!.write(to: source)
        let badDirectory = SignatureAssetStore(directory: URL(string: "https://example.com/library")!)
        XCTAssertThrowsError(try badDirectory.reload())
        let store = SignatureAssetStore(directory: root.appendingPathComponent("store"))
        XCTAssertThrowsError(try store.importPNG(from: URL(string: "https://example.com/image.png")!))
        let second = root.appendingPathComponent("second.png")
        rep.setColor(NSColor(deviceRed: 1, green: 0, blue: 0, alpha: 1), atX: 0, y: 0)
        try rep.representation(using: .png, properties: [:])!.write(to: second)
        let failDirectory = root.appendingPathComponent("fail")
        let seeded = SignatureAssetStore(directory: failDirectory)
        _ = try seeded.importPNG(from: source)
        let originalIndex = try Data(contentsOf: failDirectory.appendingPathComponent("index.json"))
        let failing = SignatureAssetStore(directory: failDirectory, failIndexWrite: { true })
        XCTAssertThrowsError(try failing.importPNG(from: second))
        XCTAssertEqual(try Data(contentsOf: failDirectory.appendingPathComponent("index.json")), originalIndex)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: failDirectory.path).count, 2)
        let budget = try SignatureImage(pngData: Data(contentsOf: source)).pngData.count
        print("Signature byte-budget test: \(budget) bytes")
        let limited = SignatureAssetStore(directory: root.appendingPathComponent("limited"), byteLimit: budget)
        _ = try limited.importPNG(from: source)
        XCTAssertThrowsError(try limited.importPNG(from: second))
        XCTAssertEqual(limited.entries.count, 1)
        let unlinkFail = SignatureAssetStore(directory: root.appendingPathComponent("unlink"), byteLimit: 128 * 1024 * 1024, failUnlink: { true })
        let asset = try unlinkFail.importPNG(from: source)
        XCTAssertThrowsError(try unlinkFail.remove(id: asset.id))
        XCTAssertNotNil(unlinkFail.lastError)
        XCTAssertTrue(unlinkFail.entries.isEmpty)
        XCTAssertTrue(FileManager.default.fileExists(atPath: root.appendingPathComponent("unlink/\(asset.id.uuidString).png").path))
    }

    @MainActor func testHundredDistinctEntriesRejectsNext() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = SignatureAssetStore(directory: root.appendingPathComponent("store"))
        let source = root.appendingPathComponent("source.png")
        for i in 0...100 {
            let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 1, pixelsHigh: 1, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
            rep.setColor(NSColor(deviceRed: CGFloat(i) / 255, green: 0, blue: 0, alpha: 1), atX: 0, y: 0)
            try rep.representation(using: .png, properties: [:])!.write(to: source)
            if i < 100 { _ = try store.importPNG(from: source) }
            else { XCTAssertThrowsError(try store.importPNG(from: source)) }
        }
        XCTAssertEqual(store.entries.count, 100)
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
