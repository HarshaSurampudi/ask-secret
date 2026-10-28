import Foundation

/// One secret to collect, as sent by the `ask-secret` core.
struct Field: Identifiable, Decodable {
    let key: String
    var label: String
    var help: String?
    var pattern: String?
    var hint: String?
    var placeholder: String?
    var optional = false
    /// The target already has a value; leaving the field empty keeps it.
    var existing = false
    /// `shared` has this key, so the user can link to it instead of typing.
    var sharedAvailable = false
    var id: String { key }

    enum CodingKeys: String, CodingKey {
        case key, label, help, pattern, hint, placeholder, optional, existing
        case sharedAvailable = "shared_available"
    }

    /// Paste mistakes worth a second look. None of these block saving; the core has
    /// the same checks for its terminal prompt.
    func warnings(_ value: String, others: [(String, String)]) -> [String] {
        var w: [String] = []
        if value.count >= 2, (value.hasPrefix("\"") && value.hasSuffix("\"")) || (value.hasPrefix("'") && value.hasSuffix("'")) {
            w.append("It's wrapped in quotes; the quotes will be part of the value.")
        }
        if value.hasPrefix("\(key)=") { w.append("It starts with \"\(key)=\"; paste only the value.") }
        if value.contains(where: \.isWhitespace) { w.append("It contains spaces or line breaks.") }
        let lower = value.lowercased()
        if ["your-", "your_", "<", "xxxx", "changeme", "placeholder", "example"].contains(where: lower.contains) {
            w.append("It looks like placeholder text.")
        }
        if let other = others.first(where: { $0.1 == value }) { w.append("It's the same as \(other.0).") }
        if let pattern, value.range(of: "^(?:\(pattern))$", options: .regularExpression) == nil,
           (try? NSRegularExpression(pattern: pattern)) != nil {
            w.append(hint ?? "It doesn't look like the usual format.")
        }
        return w
    }
}

struct Spec: Decodable {
    var title: String
    var message: String?
    var destination: String
    var canShare: Bool
    var timeout: TimeInterval
    var fields: [Field]

    enum CodingKeys: String, CodingKey {
        case title, message, destination, timeout, fields
        case canShare = "can_share"
    }
}
