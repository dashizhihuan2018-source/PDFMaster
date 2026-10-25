import AppKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor private final class ReadCountingSecrets: AccountSecretStorage {
    var reads = 0
    func read(service: String, account: String) throws -> String? { reads += 1; return "never-render-this-secret" }
    func save(_ value: String, service: String, account: String) throws {}
    func delete(service: String, account: String) throws {}
}
@MainActor final class ProviderSettingsTests: XCTestCase {
    func testDeepSeekIsIsolatedAcrossAccountsAndProvidersAndNeverWrittenToMetadata() throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).appendingPathComponent("accounts.json")
        let store = AccountStore(metadataURL: url, secretStorage: MemorySecretStorage())
        XCTAssertThrowsError(try store.readDeepSeekKey())
        for user in ["First", "Second"] { try store.register(username: user, password: "fixture-password") }
        try store.login(username: "First", password: "fixture-password", remember: false)
        try store.saveDMXKey("fixture-dmx"); try store.saveAgnesKey("fixture-agnes")
        XCTAssertNil(try store.readDeepSeekKey())
        for invalid in ["", "\n", "invalid\nheader", String(repeating: "x", count: 4097)] { XCTAssertThrowsError(try store.saveDeepSeekKey(invalid)) }
        try store.saveDeepSeekKey("fixture-deepseek-first")
        XCTAssertTrue(try store.hasSavedKey(for: .deepseek))
        XCTAssertFalse(String(decoding: try Data(contentsOf: url), as: UTF8.self).contains("fixture-deepseek-first"))
        store.logout(); try store.login(username: "Second", password: "fixture-password", remember: false)
        XCTAssertNil(try store.readDeepSeekKey()); XCTAssertFalse(try store.hasSavedKey(for: .deepseek))
        try store.saveDeepSeekKey("fixture-deepseek-second")
        store.logout(); try store.login(username: "First", password: "fixture-password", remember: false)
        XCTAssertEqual(try store.readDeepSeekKey(), "fixture-deepseek-first")
        try store.deleteDeepSeekKey()
        XCTAssertFalse(try store.hasSavedKey(for: .deepseek)); XCTAssertEqual(try store.readDMXKey(), "fixture-dmx"); XCTAssertEqual(try store.readAgnesKey(), "fixture-agnes")
        store.logout(); try store.login(username: "Second", password: "fixture-password", remember: false)
        XCTAssertEqual(try store.readDeepSeekKey(), "fixture-deepseek-second")
    }
    func testDeepSeekSettingsSaveRemoveAndClearSecureFields() throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).appendingPathComponent("accounts.json")
        let store = AccountStore(metadataURL: url, secretStorage: MemorySecretStorage())
        try store.register(username: "ProviderTest", password: "fixture-password")
        try store.login(username: "ProviderTest", password: "fixture-password", remember: false)
        let c = AccountSettingsController(store: store); _ = c.view
        c.deepSeekKeyField.stringValue = "fixture-deepseek"
        c.performSaveDeepSeekKey()
        XCTAssertEqual(try store.readDeepSeekKey(), "fixture-deepseek"); XCTAssertEqual(c.deepSeekKeyField.stringValue, "")
        XCTAssertTrue(c.deepSeekKeyStatus.stringValue.contains("已保存"))
        c.performRemoveDeepSeekKey(); XCTAssertNil(try store.readDeepSeekKey())
        c.deepSeekKeyField.stringValue = "unsaved-fixture"; c.performLogout()
        XCTAssertEqual(c.deepSeekKeyField.stringValue, ""); XCTAssertFalse(c.deepSeekKeyField.isEnabled)
    }
    func testValidationProfilePresenceProbeCannotSeeFormalSecrets() throws {
        let base = MemorySecretStorage()
        try base.save("formal-fixture", service: "service", account: "account")
        let isolated = ProfileAccountSecretStorage(base: base, namespace: "test-validation")
        XCTAssertFalse(try isolated.contains(service: "service", account: "account"))
        try isolated.save("isolated-fixture", service: "service", account: "account")
        XCTAssertTrue(try isolated.contains(service: "service", account: "account"))
        XCTAssertEqual(try base.read(service: "service", account: "account"), "formal-fixture")
    }
    func testOpeningSettingsDoesNotDecryptAnyProviderKey() throws {
        let secrets = ReadCountingSecrets()
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).appendingPathComponent("accounts.json")
        let store = AccountStore(metadataURL: url, secretStorage: secrets)
        try store.register(username: "ProviderTest", password: "provider-test-password")
        try store.login(username: "ProviderTest", password: "provider-test-password", remember: false)
        let controller = AccountSettingsController(store: store); _ = controller.view
        XCTAssertEqual(secrets.reads, 0, "Status refresh must not retrieve secret values or trigger authorization")
        XCTAssertFalse(controller.keyStatus.stringValue.contains("never-render-this-secret"))
    }
    func testDeepSeekHasItsOwnSecureSettingControl() throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).appendingPathComponent("accounts.json")
        let controller = AccountSettingsController(store: AccountStore(metadataURL: url, secretStorage: MemorySecretStorage())); _ = controller.view
        func fields(_ v: NSView) -> [NSTextField] { (v as? NSTextField).map { [$0] } ?? v.subviews.flatMap(fields) }
        let field = try XCTUnwrap(fields(controller.view).first { $0.accessibilityLabel() == "DeepSeek API Key" })
        XCTAssertTrue(field is NSSecureTextField); XCTAssertFalse(field.isEnabled)
    }
}
