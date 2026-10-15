import XCTest
import AppKit
@testable import PaperDesk

@MainActor final class ToolIconTests: XCTestCase {
    func testGeneratedIconLoadsAsFullColorAndUnknownIDsAreRejected() throws {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let assets = root.appendingPathComponent("Resources/ToolIcons")
        let icon = try XCTUnwrap(ToolIcon.load(id: "annotate", directory: assets))
        XCTAssertFalse(icon.isTemplate)
        XCTAssertGreaterThan(icon.size.width, 0)
        XCTAssertNil(ToolIcon.load(id: "../../not-a-tool", directory: assets))
        XCTAssertNil(ToolIcon.load(id: "annotate", directory: root.appendingPathComponent("missing")))
    }

    func testCompleteSkinsPersistAndRejectPartialSetsWithoutMixing() throws {
        let root = try skinFixture()
        let suite = "pdfmaster.skin.tests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        addTeardownBlock { defaults.removePersistentDomain(forName: suite) }
        let store = ToolIconSkinStore(defaults: defaults, resourceRoot: root, toolIDs: ToolCatalog.all.map(\.id))

        XCTAssertEqual(store.selected, .artMinimal)
        XCTAssertTrue(store.isAvailable(.artMinimal))
        XCTAssertTrue(store.isAvailable(.classic))
        try store.select(.classic)
        XCTAssertEqual(defaults.string(forKey: ToolIconSkinStore.preferenceKey), "classic")
        XCTAssertEqual(ToolIconSkinStore(defaults: defaults, resourceRoot: root, toolIDs: ToolCatalog.all.map(\.id)).selected, .classic)

        try FileManager.default.removeItem(at: root.appendingPathComponent("ToolSkins/art-minimal/edit.png"))
        defaults.set("art-minimal", forKey: ToolIconSkinStore.preferenceKey)
        let fallback = ToolIconSkinStore(defaults: defaults, resourceRoot: root, toolIDs: ToolCatalog.all.map(\.id))
        XCTAssertEqual(fallback.selected, .classic)
        XCTAssertThrowsError(try fallback.select(.artMinimal))
        XCTAssertNotNil(fallback.image(id: "edit"))
        XCTAssertNil(fallback.image(id: "../edit"))
    }

    func testSkinButtonOffersArtAndClassicChoices() throws {
        _ = NSApplication.shared
        let root = try skinFixture()
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "pdfmaster.skin.button.\(UUID().uuidString)"))
        let store = ToolIconSkinStore(defaults: defaults, resourceRoot: root, toolIDs: ToolCatalog.all.map(\.id))
        let button = ToolIconSkinButton(store: store)
        let choices = button.makePickerContent().subviews.flatMap(\.subviews).compactMap { $0 as? NSButton }
        XCTAssertEqual(Set(choices.compactMap { $0.identifier?.rawValue }), Set(["art-minimal", "classic"]))
        XCTAssertEqual(button.accessibilityLabel(), "切换整套工具图标皮肤")
    }

    private func skinFixture() throws -> URL {
        let repository = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ToolSkinFixture-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root.appendingPathComponent("ToolSkins"), withIntermediateDirectories: true)
        try FileManager.default.copyItem(at: repository.appendingPathComponent("Resources/ToolIcons"), to: root.appendingPathComponent("ToolIcons"))
        try FileManager.default.copyItem(at: repository.appendingPathComponent("Resources/ToolSkins/art-minimal"), to: root.appendingPathComponent("ToolSkins/art-minimal"))
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        return root
    }
}
