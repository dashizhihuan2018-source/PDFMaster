import Foundation
import CryptoKit
import CommonCrypto
import Security
import Darwin
import LocalAuthentication

@MainActor
protocol AccountSecretStorage {
    func read(service: String, account: String) throws -> String?
    func save(_ value: String, service: String, account: String) throws
    func delete(service: String, account: String) throws
}

@MainActor
final class KeychainAccountSecretStorage: AccountSecretStorage {
    private func query(service: String, account: String) -> [String: Any] {
        let context = LAContext(); context.interactionNotAllowed = true
        return [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service,
         kSecAttrAccount as String: account, kSecAttrSynchronizable as String: false,
         kSecUseAuthenticationContext as String: context]
    }
    func read(service: String, account: String) throws -> String? {
        var request = query(service: service, account: account)
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let bytes = result as? Data,
              let value = String(data: bytes, encoding: .utf8) else { throw AccountStoreFailure.secureStorageUnavailable }
        return value
    }
    func save(_ value: String, service: String, account: String) throws {
        let request = query(service: service, account: account)
        let attributes: [String: Any] = [kSecValueData as String: Data(value.utf8)]
        let update = SecItemUpdate(request as CFDictionary, attributes as CFDictionary)
        if update == errSecSuccess { return }
        guard update == errSecItemNotFound else { throw AccountStoreFailure.secureStorageUnavailable }
        var addition = request.merging(attributes) { _, new in new }
        addition[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        guard SecItemAdd(addition as CFDictionary, nil) == errSecSuccess else { throw AccountStoreFailure.secureStorageUnavailable }
    }
    func delete(service: String, account: String) throws {
        let status = SecItemDelete(query(service: service, account: account) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw AccountStoreFailure.secureStorageUnavailable }
    }
}

enum AccountStoreFailure: Error, Equatable, LocalizedError {
    case invalidCredentials, loginRequired, invalidUsername, duplicateUsername, invalidPassword, unchangedPassword, invalidKey
    case corruptMetadata, persistenceUnavailable, secureStorageUnavailable, cryptographyUnavailable
    var errorDescription: String? {
        switch self {
        case .invalidCredentials: return "用户名或密码不正确。"
        case .loginRequired: return "请先登录本机账户。"
        case .invalidUsername: return "用户名需为 2–64 位字母、数字、下划线、短横线或点。"
        case .duplicateUsername: return "这个本机用户名已存在。"
        case .invalidPassword: return "密码需为 8–1024 字节，且不能为空白。"
        case .unchangedPassword: return "新密码不能与当前密码相同。"
        case .invalidKey: return "请输入有效的 DMX Key（不可包含空白或控制字符）。"
        case .corruptMetadata: return "本机账户文件异常；已停止登录，原文件未被覆盖。"
        case .persistenceUnavailable: return "无法安全保存本机账户设置，请检查本机目录权限。"
        case .secureStorageUnavailable: return "macOS 钥匙串暂不可用，请解锁登录钥匙串后再试。"
        case .cryptographyUnavailable: return "本机安全密码计算不可用。"
        }
    }
}

/// Local convenience accounts, not operating-system users, remote identities or a
/// security boundary against another process running as the same macOS user.
/// Passwords use salted PBKDF2-HMAC-SHA256 (600,000 rounds); DMX keys and session
/// bearer secrets live only in the non-synchronizing, device-local Keychain.
@MainActor
final class AccountStore {
    static let shared = AccountStore()
    static let didChangeNotification = Notification.Name("PDFMasterAccountDidChange")
    static let dmxKeychainService = "cn.nankong.pdfmaster.ai"
    private static let sessionService = "cn.nankong.pdfmaster.session"
    private static let bootstrapPassword = "ZJKG8888"
    private static let rounds = 600_000
    private(set) var currentUsername: String?
    private(set) var initializationError: Error?
    private(set) var lastOperationError: Error?
    var isLoggedIn: Bool { currentUsername != nil }
    var rememberedUsername: String? { metadata.rememberedUsername }
    var keepsLogin: Bool { metadata.sessionVerifier != nil && isLoggedIn }
    var isUsingDefaultPassword: Bool { currentAccount?.usesDefaultPassword ?? false }
    private let metadataURL: URL
    private let secretStorage: AccountSecretStorage
    private var metadata = Metadata(accounts: [])

    private struct Account: Codable {
        var username: String
        var salt: Data
        var passwordHash: Data
        var rounds: Int
        var usesDefaultPassword: Bool
    }
    private struct Metadata: Codable {
        var version = 1
        var accounts: [Account]
        var rememberedUsername: String?
        var sessionVerifier: Data?
    }
    private var currentAccount: Account? { metadata.accounts.first { $0.username == currentUsername } }

    convenience init() {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        self.init(metadataURL: support.appendingPathComponent("cn.nankong.pdfmaster/Accounts/accounts-v1.json"),
                  secretStorage: KeychainAccountSecretStorage())
    }

    init(metadataURL: URL, secretStorage: AccountSecretStorage) {
        self.metadataURL = metadataURL; self.secretStorage = secretStorage
        do {
            var info = stat()
            let found = lstat(metadataURL.path, &info)
            if found != 0 && errno == ENOENT {
                // Public bootstrap password requested by the user. Never persisted as plaintext.
                metadata.accounts = [try Self.makeAccount(username: "Nankong", password: Self.bootstrapPassword)]
                return
            }
            guard found == 0, (info.st_mode & S_IFMT) == S_IFREG, info.st_size > 0, info.st_size <= 1_048_576 else {
                throw AccountStoreFailure.corruptMetadata
            }
            metadata = try JSONDecoder().decode(Metadata.self, from: Data(contentsOf: metadataURL))
            guard metadata.version == 1, !metadata.accounts.isEmpty, metadata.accounts.count <= 100,
                  metadata.accounts.allSatisfy({ Self.normalizedUsername($0.username) != nil && $0.salt.count == 16 && $0.passwordHash.count == 32 && $0.rounds == Self.rounds }),
                  Set(metadata.accounts.compactMap { Self.normalizedUsername($0.username) }).count == metadata.accounts.count else {
                throw AccountStoreFailure.corruptMetadata
            }
            if let remembered = metadata.rememberedUsername {
                guard metadata.accounts.contains(where: { $0.username == remembered }) else { throw AccountStoreFailure.corruptMetadata }
            }
            if let verifier = metadata.sessionVerifier {
                guard verifier.count == 32, let username = metadata.rememberedUsername else { throw AccountStoreFailure.corruptMetadata }
                if let token = try secretStorage.read(service: Self.sessionService, account: Self.normalizedUsername(username)!),
                   Self.constantTimeEqual(Self.sessionHash(token), verifier) { currentUsername = username }
            }
        } catch {
            // A locked keychain does not corrupt account metadata; password login remains possible.
            if error as? AccountStoreFailure == .secureStorageUnavailable { lastOperationError = error }
            else { initializationError = AccountStoreFailure.corruptMetadata; metadata = Metadata(accounts: []) }
            currentUsername = nil
        }
    }

    func login(username: String, password: String, remember: Bool) throws {
        try requireHealthyMetadata()
        guard let normalized = Self.normalizedUsername(username), let account = metadata.accounts.first(where: { Self.normalizedUsername($0.username) == normalized }),
              password.utf8.count <= 1024,
              Self.constantTimeEqual(try Self.derive(password: password, salt: account.salt), account.passwordHash) else {
            throw AccountStoreFailure.invalidCredentials
        }
        var next = metadata
        next.rememberedUsername = account.username; next.sessionVerifier = nil
        if let previous = metadata.rememberedUsername, metadata.sessionVerifier != nil {
            try secretStorage.delete(service: Self.sessionService, account: Self.normalizedUsername(previous)!)
        }
        if remember {
            let token = try Self.randomBytes(count: 32).base64EncodedString()
            try secretStorage.save(token, service: Self.sessionService, account: normalized)
            next.sessionVerifier = Self.sessionHash(token)
        }
        do { try persist(next) }
        catch {
            if remember { try? secretStorage.delete(service: Self.sessionService, account: normalized) }
            throw error
        }
        metadata = next; currentUsername = account.username; lastOperationError = nil; notify()
    }

    func logout() {
        let previous = currentUsername ?? metadata.rememberedUsername
        currentUsername = nil
        var next = metadata; next.sessionVerifier = nil
        lastOperationError = nil
        // Revoke the Keychain bearer even when a filesystem write fails.
        if let previous, metadata.sessionVerifier != nil {
            do { try secretStorage.delete(service: Self.sessionService, account: Self.normalizedUsername(previous)!) }
            catch { lastOperationError = error }
        }
        if initializationError == nil {
            do { try persist(next) } catch { lastOperationError = error }
        }
        metadata = next; notify()
    }

    func readDMXKey() throws -> String? {
        let username = try loggedInKey()
        return try secretStorage.read(service: Self.dmxKeychainService, account: username)
    }
    func saveDMXKey(_ value: String) throws {
        let username = try loggedInKey()
        guard !value.isEmpty, value.utf8.count <= 4096,
              !value.unicodeScalars.contains(where: { CharacterSet.whitespacesAndNewlines.union(.controlCharacters).contains($0) }) else { throw AccountStoreFailure.invalidKey }
        try secretStorage.save(value, service: Self.dmxKeychainService, account: username)
        notify()
    }
    func deleteDMXKey() throws {
        try secretStorage.delete(service: Self.dmxKeychainService, account: loggedInKey())
        notify()
    }

    func changePassword(old: String, new: String) throws {
        let normalized = try loggedInKey()
        guard let index = metadata.accounts.firstIndex(where: { $0.username == currentUsername }), old.utf8.count <= 1024,
              Self.constantTimeEqual(try Self.derive(password: old, salt: metadata.accounts[index].salt), metadata.accounts[index].passwordHash) else { throw AccountStoreFailure.invalidCredentials }
        try Self.validatePassword(new)
        guard old != new else { throw AccountStoreFailure.unchangedPassword }
        var next = metadata
        next.accounts[index] = try Self.makeAccount(username: metadata.accounts[index].username, password: new)
        if next.sessionVerifier != nil {
            let token = try Self.randomBytes(count: 32).base64EncodedString()
            try secretStorage.save(token, service: Self.sessionService, account: normalized)
            next.sessionVerifier = Self.sessionHash(token)
        }
        try persist(next); metadata = next; notify()
    }

    func register(username: String, password: String) throws {
        try requireHealthyMetadata()
        guard let normalized = Self.normalizedUsername(username), metadata.accounts.count < 100 else { throw AccountStoreFailure.invalidUsername }
        guard !metadata.accounts.contains(where: { Self.normalizedUsername($0.username) == normalized }) else { throw AccountStoreFailure.duplicateUsername }
        try Self.validatePassword(password)
        var next = metadata
        next.accounts.append(try Self.makeAccount(username: username.trimmingCharacters(in: .whitespacesAndNewlines), password: password))
        try persist(next); metadata = next; notify()
    }

    private func requireHealthyMetadata() throws { if let initializationError { throw initializationError } }
    private func loggedInKey() throws -> String {
        try requireHealthyMetadata()
        guard let username = currentUsername, let normalized = Self.normalizedUsername(username) else { throw AccountStoreFailure.loginRequired }
        return normalized
    }
    private func notify() { NotificationCenter.default.post(name: Self.didChangeNotification, object: self) }
    private static func normalizedUsername(_ value: String) -> String? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard (2...64).contains(trimmed.utf8.count), trimmed.range(of: "^[A-Za-z0-9_.-]+$", options: .regularExpression) != nil else { return nil }
        return trimmed.lowercased()
    }
    private static func validatePassword(_ value: String) throws {
        guard (8...1024).contains(value.utf8.count), !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw AccountStoreFailure.invalidPassword }
    }
    private static func makeAccount(username: String, password: String) throws -> Account {
        let salt = try randomBytes(count: 16)
        return Account(username: username, salt: salt, passwordHash: try derive(password: password, salt: salt), rounds: rounds, usesDefaultPassword: password == bootstrapPassword)
    }
    private static func randomBytes(count: Int) throws -> Data {
        var bytes = [UInt8](repeating: 0, count: count)
        guard SecRandomCopyBytes(kSecRandomDefault, count, &bytes) == errSecSuccess else { throw AccountStoreFailure.cryptographyUnavailable }
        return Data(bytes)
    }
    private static func derive(password: String, salt: Data) throws -> Data {
        var output = [UInt8](repeating: 0, count: 32)
        let passwordBytes = Array(password.utf8)
        let result = passwordBytes.withUnsafeBytes { passwordBuffer in
            salt.withUnsafeBytes { saltBuffer in
                CCKeyDerivationPBKDF(CCPBKDFAlgorithm(kCCPBKDF2), passwordBuffer.baseAddress?.assumingMemoryBound(to: Int8.self), passwordBytes.count,
                    saltBuffer.baseAddress?.assumingMemoryBound(to: UInt8.self), salt.count, CCPseudoRandomAlgorithm(kCCPRFHmacAlgSHA256), UInt32(rounds), &output, output.count)
            }
        }
        guard result == kCCSuccess else { throw AccountStoreFailure.cryptographyUnavailable }
        return Data(output)
    }
    private static func sessionHash(_ token: String) -> Data { Data(SHA256.hash(data: Data(token.utf8))) }
    private static func constantTimeEqual(_ left: Data, _ right: Data) -> Bool {
        guard left.count == right.count else { return false }
        return zip(left, right).reduce(UInt8(0)) { $0 | ($1.0 ^ $1.1) } == 0
    }
    private func persist(_ next: Metadata) throws {
        let directory = metadataURL.deletingLastPathComponent()
        let temporary = directory.appendingPathComponent(".accounts-" + UUID().uuidString)
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            let bytes = try JSONEncoder().encode(next)
            let descriptor = Darwin.open(temporary.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
            guard descriptor >= 0 else { throw AccountStoreFailure.persistenceUnavailable }
            let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
            defer { try? handle.close(); try? FileManager.default.removeItem(at: temporary) }
            try handle.write(contentsOf: bytes)
            guard fsync(descriptor) == 0, Darwin.rename(temporary.path, metadataURL.path) == 0 else { throw AccountStoreFailure.persistenceUnavailable }
        } catch { throw AccountStoreFailure.persistenceUnavailable }
    }
}
