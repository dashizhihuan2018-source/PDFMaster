import Foundation
import XCTest
@testable import PaperDesk

@MainActor
final class MemorySecretStorage: AccountSecretStorage {
    private var entries: [String: String] = [:]
    func contains(service: String, account: String) throws -> Bool { entries[service + ":" + account] != nil }
    func read(service: String, account: String) throws -> String? { entries[service + ":" + account] }
    func save(_ value: String, service: String, account: String) throws { entries[service + ":" + account] = value }
    func delete(service: String, account: String) throws { entries.removeValue(forKey: service + ":" + account) }
    func snapshot() -> MemorySecretStorage {
        let copy = MemorySecretStorage(); copy.entries = entries; return copy
    }
}

@MainActor
final class AccountStoreTests: XCTestCase {
    private func fixture() -> (URL, MemorySecretStorage, AccountStore) {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-AccountTests-" + UUID().uuidString).appendingPathComponent("accounts.json")
        let secrets = MemorySecretStorage()
        return (url, secrets, AccountStore(metadataURL: url, secretStorage: secrets))
    }

    func testDefaultLoginRejectsWrongPasswordAndNormalizesUsernameOnly() throws {
        let (_, _, store) = fixture()
        XCTAssertFalse(store.isLoggedIn)
        XCTAssertThrowsError(try store.login(username: "Nankong", password: "incorrect", remember: false))
        XCTAssertFalse(store.isLoggedIn)
        try store.login(username: "  nAnKoNg \n", password: "ZJKG8888", remember: false)
        XCTAssertTrue(store.isLoggedIn)
        XCTAssertEqual(store.currentUsername, "Nankong")
        store.logout()
        XCTAssertThrowsError(try store.login(username: "Nankong", password: " ZJKG8888 ", remember: false))
    }

    func testRememberedLoginRestoresOnlyWhenSecureSessionSecretExists() throws {
        let (url, secrets, store) = fixture()
        try store.login(username: "Nankong", password: "ZJKG8888", remember: true)
        XCTAssertEqual(AccountStore(metadataURL: url, secretStorage: secrets).currentUsername, "Nankong")
        XCTAssertFalse(AccountStore(metadataURL: url, secretStorage: MemorySecretStorage()).isLoggedIn)
        store.logout()
        XCTAssertFalse(AccountStore(metadataURL: url, secretStorage: secrets).isLoggedIn)
    }

    func testNonRememberedLoginDoesNotRestoreAfterRecreation() throws {
        let (url, secrets, store) = fixture()
        try store.login(username: "Nankong", password: "ZJKG8888", remember: false)
        XCTAssertFalse(AccountStore(metadataURL: url, secretStorage: secrets).isLoggedIn)
        XCTAssertEqual(AccountStore(metadataURL: url, secretStorage: secrets).rememberedUsername, "Nankong")
    }

    func testPerAccountKeyIsolationAndDeletion() throws {
        let (_, _, store) = fixture()
        XCTAssertThrowsError(try store.readDMXKey())
        XCTAssertThrowsError(try store.saveDMXKey("first-test-secret"))
        try store.login(username: "Nankong", password: "ZJKG8888", remember: false)
        try store.saveDMXKey("first-test-secret")
        store.logout()
        try store.register(username: "Second", password: "second-password")
        try store.login(username: "second", password: "second-password", remember: false)
        XCTAssertNil(try store.readDMXKey())
        try store.saveDMXKey("second-test-secret")
        store.logout()
        try store.login(username: "nankong", password: "ZJKG8888", remember: false)
        XCTAssertEqual(try store.readDMXKey(), "first-test-secret")
        try store.deleteDMXKey()
        XCTAssertNil(try store.readDMXKey())
        store.logout()
        try store.login(username: "second", password: "second-password", remember: false)
        XCTAssertEqual(try store.readDMXKey(), "second-test-secret")
    }

    func testAgnesKeyIsIsolatedByProviderAndAccountAndSurvivesRecreation() throws {
        let (url, secrets, store) = fixture()
        XCTAssertThrowsError(try store.readAgnesKey())
        XCTAssertThrowsError(try store.saveAgnesKey("first-agnes-test-secret"))
        XCTAssertThrowsError(try store.deleteAgnesKey())
        try store.login(username: "Nankong", password: "ZJKG8888", remember: true)
        try store.saveDMXKey("first-dmx-test-secret")
        XCTAssertNil(try store.readAgnesKey(), "DMX must never be used as an Agnes fallback")
        try store.saveAgnesKey("first-agnes-test-secret")
        XCTAssertEqual(try secrets.read(service: "cn.nankong.pdfmaster.ai.agnes", account: "nankong"), "first-agnes-test-secret")
        XCTAssertEqual(try AccountStore(metadataURL: url, secretStorage: secrets).readAgnesKey(), "first-agnes-test-secret")
        let metadata = String(decoding: try Data(contentsOf: url), as: UTF8.self)
        XCTAssertFalse(metadata.contains("first-agnes-test-secret"))
        store.logout()
        try store.register(username: "Second", password: "second-password")
        try store.login(username: "second", password: "second-password", remember: false)
        XCTAssertNil(try store.readAgnesKey())
        try store.saveAgnesKey("second-agnes-test-secret")
        store.logout()
        try store.login(username: "nankong", password: "ZJKG8888", remember: false)
        XCTAssertEqual(try store.readAgnesKey(), "first-agnes-test-secret")
        try store.deleteAgnesKey()
        XCTAssertNil(try store.readAgnesKey())
        XCTAssertEqual(try store.readDMXKey(), "first-dmx-test-secret")
        store.logout()
        try store.login(username: "second", password: "second-password", remember: false)
        XCTAssertEqual(try store.readAgnesKey(), "second-agnes-test-secret")
    }

    func testAgnesKeyRejectsHeaderInjectionAndDoesNotConsultLegacySharedSecrets() throws {
        let (_, secrets, store) = fixture()
        try secrets.save("legacy-shared-test-secret", service: "cn.nankong.pdfmaster.ai.agnes", account: "shared")
        try store.login(username: "Nankong", password: "ZJKG8888", remember: false)
        XCTAssertNil(try store.readAgnesKey())
        for invalid in ["", "   ", "invalid\nheader", "invalid\tkey", String(repeating: "x", count: 4097)] {
            XCTAssertThrowsError(try store.saveAgnesKey(invalid))
        }
        XCTAssertNil(try store.readAgnesKey())
        XCTAssertEqual(try secrets.read(service: "cn.nankong.pdfmaster.ai.agnes", account: "shared"), "legacy-shared-test-secret")
    }

    func testMetadataContainsNeitherPlaintextPasswordNorAPIKey() throws {
        let (url, _, store) = fixture()
        try store.login(username: "Nankong", password: "ZJKG8888", remember: true)
        try store.saveDMXKey("private-test-DMX-key")
        let data = try Data(contentsOf: url)
        let text = String(decoding: data, as: UTF8.self)
        XCTAssertFalse(text.contains("ZJKG8888"))
        XCTAssertFalse(text.contains("private-test-DMX-key"))
        XCTAssertFalse(text.contains("second-password"))
        let root = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let accounts = try XCTUnwrap(root["accounts"] as? [[String: Any]])
        let first = try XCTUnwrap(accounts.first)
        XCTAssertEqual(first["rounds"] as? Int, 600_000)
        XCTAssertNotNil(first["salt"])
        XCTAssertNotNil(first["passwordHash"])
        let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
        XCTAssertEqual((attributes[.posixPermissions] as? NSNumber)?.intValue, 0o600)
    }

    func testChangingPasswordInvalidatesOldPasswordAndPersistsNewHash() throws {
        let (url, secrets, store) = fixture()
        try store.login(username: "Nankong", password: "ZJKG8888", remember: true)
        XCTAssertTrue(store.isUsingDefaultPassword)
        XCTAssertThrowsError(try store.changePassword(old: "incorrect", new: "new-password"))
        XCTAssertThrowsError(try store.changePassword(old: "ZJKG8888", new: "short"))
        try store.changePassword(old: "ZJKG8888", new: "new-password")
        XCTAssertFalse(store.isUsingDefaultPassword)
        store.logout()
        let reopened = AccountStore(metadataURL: url, secretStorage: secrets)
        XCTAssertThrowsError(try reopened.login(username: "Nankong", password: "ZJKG8888", remember: false))
        try reopened.login(username: "Nankong", password: "new-password", remember: false)
        XCTAssertFalse(String(decoding: try Data(contentsOf: url), as: UTF8.self).contains("new-password"))
    }

    func testPasswordChangeRotatesRememberedSessionToken() throws {
        let (url, secrets, store) = fixture()
        try store.login(username: "Nankong", password: "ZJKG8888", remember: true)
        let oldSessionSecrets = secrets.snapshot()
        try store.changePassword(old: "ZJKG8888", new: "rotated-password")
        XCTAssertTrue(AccountStore(metadataURL: url, secretStorage: secrets).isLoggedIn)
        XCTAssertFalse(AccountStore(metadataURL: url, secretStorage: oldSessionSecrets).isLoggedIn)
    }

    func testDefaultPasswordCannotBeMarkedChangedByReusingSamePassword() throws {
        let (_, _, store) = fixture()
        try store.login(username: "Nankong", password: "ZJKG8888", remember: false)
        XCTAssertThrowsError(try store.changePassword(old: "ZJKG8888", new: "ZJKG8888"))
        XCTAssertTrue(store.isUsingDefaultPassword)
    }

    func testReturningToPublicDefaultPasswordRestoresWarning() throws {
        let (_, _, store) = fixture()
        try store.login(username: "Nankong", password: "ZJKG8888", remember: false)
        try store.changePassword(old: "ZJKG8888", new: "changed-password")
        try store.changePassword(old: "changed-password", new: "ZJKG8888")
        XCTAssertTrue(store.isUsingDefaultPassword)
    }

    func testInvalidHashParametersAndDuplicateAccountsFailClosed() throws {
        let (url, secrets, store) = fixture()
        try store.login(username: "Nankong", password: "ZJKG8888", remember: true)
        let valid = try Data(contentsOf: url)
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: valid) as? [String: Any])
        var accounts = try XCTUnwrap(object["accounts"] as? [[String: Any]])
        accounts[0]["rounds"] = 2_147_483_647
        object["accounts"] = accounts
        try JSONSerialization.data(withJSONObject: object).write(to: url)
        let damagedRounds = AccountStore(metadataURL: url, secretStorage: secrets)
        XCTAssertFalse(damagedRounds.isLoggedIn)
        XCTAssertThrowsError(try damagedRounds.login(username: "Nankong", password: "ZJKG8888", remember: true))
        object = try XCTUnwrap(JSONSerialization.jsonObject(with: valid) as? [String: Any])
        accounts = try XCTUnwrap(object["accounts"] as? [[String: Any]])
        object["accounts"] = accounts + accounts
        try JSONSerialization.data(withJSONObject: object).write(to: url)
        XCTAssertNotNil(AccountStore(metadataURL: url, secretStorage: secrets).initializationError)
    }

    func testMalformedExistingMetadataFailsClosedWithoutOverwritingFile() throws {
        let (url, secrets, _) = fixture()
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        let malformed = Data("{broken".utf8)
        try malformed.write(to: url)
        let store = AccountStore(metadataURL: url, secretStorage: secrets)
        XCTAssertFalse(store.isLoggedIn)
        XCTAssertNotNil(store.initializationError)
        XCTAssertThrowsError(try store.login(username: "Nankong", password: "ZJKG8888", remember: false))
        XCTAssertThrowsError(try store.register(username: "Second", password: "second-password"))
        XCTAssertEqual(try Data(contentsOf: url), malformed)
    }

    func testRegistrationDuplicateUsernameAndInvalidKeyAreRejected() throws {
        let (_, _, store) = fixture()
        XCTAssertThrowsError(try store.register(username: " nankong ", password: "duplicate-password"))
        XCTAssertThrowsError(try store.register(username: "../../other", password: "second-password"))
        try store.login(username: "Nankong", password: "ZJKG8888", remember: false)
        for invalid in ["", "   ", "invalid\nheader", String(repeating: "x", count: 4097)] {
            XCTAssertThrowsError(try store.saveDMXKey(invalid))
        }
        XCTAssertNil(try store.readDMXKey())
    }
}
