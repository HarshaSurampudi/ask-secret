import Foundation
import Security

/// Reads which keys already have a value, and writes new values.
enum Store {
    // MARK: existing

    static func existingKeys(in destination: Destination, keys: [String]) -> Set<String> {
        switch destination {
        case .envFile(let url):
            guard let text = try? String(contentsOf: url, encoding: .utf8) else { return [] }
            var found = Set<String>()
            for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
                guard let (key, value) = parseLine(String(line)), keys.contains(key) else { continue }
                if !value.isEmpty && value != "\"\"" && value != "''" { found.insert(key) }
            }
            return found
        case .keychain(let service):
            return Set(keys.filter { keychainExists(service: service, account: $0) })
        }
    }

    // MARK: save

    static func save(_ values: [String: String], to destination: Destination) throws {
        switch destination {
        case .envFile(let url): try upsertEnv(values, at: url)
        case .keychain(let service):
            for (key, value) in values { try keychainSet(service: service, account: key, value: value) }
        }
    }

    // MARK: .env

    private static func parseLine(_ line: String) -> (String, String)? {
        var s = line.trimmingCharacters(in: .whitespaces)
        if s.hasPrefix("#") || s.isEmpty { return nil }
        if s.hasPrefix("export ") { s = String(s.dropFirst(7)) }
        guard let eq = s.firstIndex(of: "=") else { return nil }
        let key = s[..<eq].trimmingCharacters(in: .whitespaces)
        let value = s[s.index(after: eq)...].trimmingCharacters(in: .whitespaces)
        return (key, value)
    }

    /// Quote only when needed. Single quotes are literal in dotenv (no ${} interpolation).
    private static func encode(_ value: String) -> String {
        let plain = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-./:@+=,")
        if !value.isEmpty && value.unicodeScalars.allSatisfy({ plain.contains($0) }) { return value }
        if !value.contains("'") && !value.contains("\n") { return "'\(value)'" }
        let escaped = value.replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
            .replacingOccurrences(of: "\n", with: "\\n")
        return "\"\(escaped)\""
    }

    private static func upsertEnv(_ values: [String: String], at url: URL) throws {
        let fm = FileManager.default
        try fm.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        let existing = (try? String(contentsOf: url, encoding: .utf8)) ?? ""
        var lines = existing.isEmpty ? [] : existing.components(separatedBy: "\n")
        if lines.last == "" { lines.removeLast() }
        var pending = values
        for (i, line) in lines.enumerated() {
            guard let (key, _) = parseLine(line), let value = pending.removeValue(forKey: key) else { continue }
            let prefix = line.trimmingCharacters(in: .whitespaces).hasPrefix("export ") ? "export " : ""
            lines[i] = "\(prefix)\(key)=\(encode(value))"
        }
        for key in values.keys.sorted() where pending[key] != nil {
            lines.append("\(key)=\(encode(pending[key]!))")
        }
        let data = Data((lines.joined(separator: "\n") + "\n").utf8)
        let tmp = url.deletingLastPathComponent().appendingPathComponent(".\(url.lastPathComponent).ask-secret.tmp")
        fm.createFile(atPath: tmp.path, contents: data, attributes: [.posixPermissions: 0o600])
        if fm.fileExists(atPath: url.path) {
            _ = try fm.replaceItemAt(url, withItemAt: tmp)
        } else {
            try fm.moveItem(at: tmp, to: url)
        }
        try fm.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    }

    // MARK: keychain

    private static func baseQuery(service: String, account: String) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: service,
         kSecAttrAccount as String: account]
    }

    private static func keychainExists(service: String, account: String) -> Bool {
        var q = baseQuery(service: service, account: account)
        q[kSecReturnAttributes as String] = true
        return SecItemCopyMatching(q as CFDictionary, nil) == errSecSuccess
    }

    private static func keychainSet(service: String, account: String, value: String) throws {
        let data = Data(value.utf8)
        let q = baseQuery(service: service, account: account)
        var status = SecItemUpdate(q as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var add = q
            add[kSecValueData as String] = data
            add[kSecAttrLabel as String] = "\(service) · \(account)"
            status = SecItemAdd(add as CFDictionary, nil)
        }
        guard status == errSecSuccess else {
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(status),
                          userInfo: [NSLocalizedDescriptionKey: "Keychain error \(status) for \(account)"])
        }
    }
}
