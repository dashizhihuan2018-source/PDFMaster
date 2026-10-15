import AppKit
import XCTest
@testable import PaperDesk

@MainActor
final class AccountSettingsControllerTests: XCTestCase {
    private func fixture() -> (AccountStore, AccountSettingsController) {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMaster-AccountUI-" + UUID().uuidString).appendingPathComponent("accounts.json")
        let store = AccountStore(metadataURL: url, secretStorage: MemorySecretStorage())
        let controller = AccountSettingsController(store: store)
        _ = controller.view
        return (store, controller)
    }

    func testLoggedOutControlsAreSecureLocalOnlyAndRequireLoginForKey() {
        let (_, controller) = fixture()
        XCTAssertTrue(controller.passwordField.cell is NSSecureTextFieldCell)
        XCTAssertTrue(controller.keyField.cell is NSSecureTextFieldCell)
        XCTAssertEqual(controller.usernameField.stringValue, "Nankong")
        XCTAssertFalse(controller.saveKeyButton.isEnabled)
        XCTAssertFalse(controller.removeKeyButton.isEnabled)
        XCTAssertFalse(controller.changePasswordButton.isEnabled)
        XCTAssertTrue(controller.securityNotice.stringValue.contains("公开默认密码"))
        XCTAssertTrue(controller.securityNotice.stringValue.contains("不提供系统管理员权限"))
    }

    func testLoginKeySaveAndLogoutClearSecretsFromControls() throws {
        let (store, controller) = fixture()
        controller.usernameField.stringValue = "Nankong"
        controller.passwordField.stringValue = "ZJKG8888"
        controller.performLogin()
        XCTAssertTrue(store.isLoggedIn)
        XCTAssertEqual(controller.passwordField.stringValue, "")
        XCTAssertTrue(controller.saveKeyButton.isEnabled)
        controller.keyField.stringValue = "UI-test-key"
        controller.performSaveKey()
        XCTAssertEqual(try store.readDMXKey(), "UI-test-key")
        XCTAssertEqual(controller.keyField.stringValue, "")
        XCTAssertTrue(controller.keyStatus.stringValue.contains("已保存"))
        XCTAssertFalse(controller.keyStatus.stringValue.contains("UI-test-key"))
        controller.keyField.stringValue = "unsaved-key"
        controller.oldPasswordField.stringValue = "ZJKG8888"
        controller.performLogout()
        XCTAssertFalse(store.isLoggedIn)
        XCTAssertEqual(controller.keyField.stringValue, "")
        XCTAssertEqual(controller.oldPasswordField.stringValue, "")
        XCTAssertFalse(controller.saveKeyButton.isEnabled)
    }

    func testExternalLogoutClearsSettingsSecretsAndDisablesControls() throws {
        let (store, controller) = fixture()
        try store.login(username: "Nankong", password: "ZJKG8888", remember: false)
        controller.keyField.stringValue = "unsaved-secret"
        store.logout()
        XCTAssertEqual(controller.keyField.stringValue, "")
        XCTAssertFalse(controller.changePasswordButton.isEnabled)
    }

    func testPasswordChangeAndKeyRemovalAreRealOperations() throws {
        let (store, controller) = fixture()
        try store.login(username: "Nankong", password: "ZJKG8888", remember: false)
        try store.saveDMXKey("remove-test-key")
        controller.performRemoveKey()
        XCTAssertNil(try store.readDMXKey())
        controller.oldPasswordField.stringValue = "ZJKG8888"
        controller.newPasswordField.stringValue = "changed-password"
        controller.confirmPasswordField.stringValue = "different-password"
        controller.performChangePassword()
        XCTAssertTrue(store.isUsingDefaultPassword)
        controller.confirmPasswordField.stringValue = "changed-password"
        controller.performChangePassword()
        XCTAssertFalse(store.isUsingDefaultPassword)
        XCTAssertEqual(controller.newPasswordField.stringValue, "")
        XCTAssertEqual(controller.confirmPasswordField.stringValue, "")
    }
}
