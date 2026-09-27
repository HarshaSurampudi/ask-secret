import Foundation

/// One secret to collect.
struct Field: Identifiable {
    let key: String
    var label: String
    var help: String?
    var pattern: String?
    var patternHint: String?
    var placeholder: String?
    var optional = false
    var id: String { key }

    func isValid(_ value: String) -> Bool {
        guard let pattern, !value.isEmpty else { return true }
        return value.range(of: "^(?:\(pattern))$", options: .regularExpression) != nil
    }
}

enum Destination {
    case envFile(URL)
    case keychain(service: String)

    var display: String {
        switch self {
        case .envFile(let url):
            return (url.path as NSString).abbreviatingWithTildeInPath
        case .keychain(let service):
            return "Keychain · \(service)"
        }
    }
}

struct Spec {
    var title = "Secrets needed"
    var message: String?
    var destination: Destination
    var fields: [Field]
    var timeout: TimeInterval = 900
}

struct UsageError: Error, CustomStringConvertible {
    let description: String
}

let usage = """
ask-secret: ask the user for secrets in a native popup and store them
directly, without the values passing through the caller.

USAGE
  ask-secret (--env FILE | --keychain SERVICE) [--title T] [--message M]
             --field KEY [--label L] [--help H] [--pattern REGEX] [--hint H]
                         [--placeholder P] [--optional]
             [--field KEY2 ...] [--timeout SECONDS]

  --env FILE          Upsert KEY=value lines into FILE (created 0600 if missing,
                      other lines kept, permissions forced to 0600).
  --keychain SERVICE  Store each KEY as a generic password (account=KEY).
  --field KEY         Starts a field; the options after it describe that field.
  --help H            Markdown help under the field; links are clickable.
  --pattern REGEX     Whole-value regex; Save stays disabled until it matches.
  --hint H            Message shown when the pattern doesn't match.
  --optional          Field may be left empty.
  --timeout SECONDS   Auto-cancel after this long (default 900).

OUTPUT (stdout, never values)
  KEY: saved | kept (already set, left empty) | skipped (optional, empty)
  or "cancelled". Exit 0 = saved, 1 = cancelled/timed out, 2 = usage error.

EXAMPLE
  ask-secret --env ~/app/.env --title "Telegram bot" \\
    --field TELEGRAM_BOT_TOKEN --label "Bot token" \\
    --help "From [@BotFather](https://t.me/BotFather) → /newbot" \\
    --pattern '[0-9]+:[A-Za-z0-9_-]{30,}' --hint "Looks like 123456:ABC…"
"""

func parseArguments(_ args: [String]) throws -> Spec {
    var title: String?
    var message: String?
    var destination: Destination?
    var fields: [Field] = []
    var timeout: TimeInterval = 900
    var i = 0

    func value(_ flag: String) throws -> String {
        i += 1
        guard i < args.count else { throw UsageError(description: "\(flag) needs a value") }
        return args[i]
    }
    func current(_ flag: String) throws -> Int {
        guard !fields.isEmpty else { throw UsageError(description: "\(flag) must follow --field") }
        return fields.count - 1
    }

    while i < args.count {
        let flag = args[i]
        switch flag {
        case "--env":
            let path = (try value(flag) as NSString).expandingTildeInPath
            destination = .envFile(URL(fileURLWithPath: path).standardizedFileURL)
        case "--keychain": destination = .keychain(service: try value(flag))
        case "--title": title = try value(flag)
        case "--message": message = try value(flag)
        case "--timeout":
            guard let t = TimeInterval(try value(flag)) else { throw UsageError(description: "--timeout needs seconds") }
            timeout = t
        case "--field":
            let key = try value(flag)
            guard key.range(of: "^[A-Za-z_][A-Za-z0-9_.-]*$", options: .regularExpression) != nil else {
                throw UsageError(description: "invalid key \(key)")
            }
            fields.append(Field(key: key, label: key))
        case "--label": let n = try current(flag); fields[n].label = try value(flag)
        case "--help": let n = try current(flag); fields[n].help = try value(flag)
        case "--pattern": let n = try current(flag); fields[n].pattern = try value(flag)
        case "--hint": let n = try current(flag); fields[n].patternHint = try value(flag)
        case "--placeholder": let n = try current(flag); fields[n].placeholder = try value(flag)
        case "--optional": let n = try current(flag); fields[n].optional = true
        case "-h", "--usage": throw UsageError(description: usage)
        default: throw UsageError(description: "unknown option \(flag)\n\n\(usage)")
        }
        i += 1
    }
    guard let destination else { throw UsageError(description: "one of --env or --keychain is required\n\n\(usage)") }
    guard !fields.isEmpty else { throw UsageError(description: "at least one --field is required\n\n\(usage)") }
    return Spec(title: title ?? (fields.count == 1 ? fields[0].label : "Secrets needed"),
                message: message, destination: destination, fields: fields, timeout: timeout)
}
