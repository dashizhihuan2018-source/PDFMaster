import Foundation

/// Isolated validation profiles must never reuse the installed app's Keychain services.
@MainActor final class ProfileAccountSecretStorage: AccountSecretStorage {
    private let base: AccountSecretStorage
    private let namespace: String
    init(base: AccountSecretStorage, namespace: String) { self.base = base; self.namespace = namespace }
    private func scoped(_ service: String) -> String { namespace + "." + service }
    func read(service: String, account: String) throws -> String? { try base.read(service: scoped(service), account: account) }
    func save(_ value: String, service: String, account: String) throws { try base.save(value, service: scoped(service), account: account) }
    func delete(service: String, account: String) throws { try base.delete(service: scoped(service), account: account) }
}
