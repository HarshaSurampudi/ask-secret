import AppKit
import SwiftUI

enum Outcome {
    case saved([String: [String: Any]])  // key -> {"action": "value"|"link"|"keep"|"skip", ...}
    case cancelled(String)
}

@Observable
final class FormModel {
    let spec: Spec
    var values: [String: String]
    /// Fields answered with the shared value instead of a new one.
    var useShared: Set<String>
    /// New values that should also go to shared, for other projects.
    var toShared: Set<String> = []
    var revealed: Set<String> = []
    var pastedFromClipboard = false
    var clearClipboard = true
    var saving = false
    var saved = false

    init(spec: Spec) {
        self.spec = spec
        self.values = Dictionary(uniqueKeysWithValues: spec.fields.map { ($0.key, "") })
        self.useShared = Set(spec.fields.filter { $0.sharedAvailable && !$0.existing }.map(\.key))
    }

    func value(_ key: String) -> String {
        (values[key] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func state(of field: Field) -> FieldState {
        if useShared.contains(field.key) { return .linked }
        let v = value(field.key)
        if v.isEmpty {
            if field.existing { return .kept }
            return field.optional ? .optionalEmpty : .missing
        }
        return warnings(for: field).isEmpty ? .ready : .warned
    }

    func warnings(for field: Field) -> [String] {
        let v = value(field.key)
        guard !v.isEmpty, !useShared.contains(field.key) else { return [] }
        let others = spec.fields.filter { $0.key != field.key && !useShared.contains($0.key) }
            .map { ($0.key, value($0.key)) }.filter { !$0.1.isEmpty }
        return field.warnings(v, others: others)
    }

    var canSave: Bool {
        let states = spec.fields.map(state(of:))
        return !saving && !states.contains(.missing) && states.contains { [.ready, .warned, .linked].contains($0) }
    }

    func save() -> Outcome? {
        guard canSave else { return nil }
        saving = true
        var answers: [String: [String: Any]] = [:]
        for f in spec.fields {
            switch state(of: f) {
            case .ready, .warned:
                answers[f.key] = ["action": "value", "value": value(f.key), "to_shared": toShared.contains(f.key)]
            case .linked: answers[f.key] = ["action": "link"]
            case .kept: answers[f.key] = ["action": "keep"]
            default: answers[f.key] = ["action": "skip"]
            }
        }
        if pastedFromClipboard && clearClipboard { NSPasteboard.general.clearContents() }
        return .saved(answers)
    }
}

enum FieldState { case missing, ready, warned, linked, kept, optionalEmpty }

// MARK: - Views

struct SecretForm: View {
    @Bindable var model: FormModel
    let finish: (Outcome) -> Void
    @FocusState private var focused: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
                .padding(.horizontal, 22)
                .padding(.top, 26)
                .padding(.bottom, 16)

            Group {
                if model.spec.fields.count > 4 {
                    ScrollView { fieldList }.frame(height: 460)
                } else {
                    fieldList
                }
            }

            footer
                .padding(.horizontal, 22)
                .padding(.vertical, 16)
        }
        .frame(width: 500)
        .background(.regularMaterial)
        .overlay(alignment: .center) { if model.saved { SavedBadge() } }
        .onAppear {
            focused = model.spec.fields.first { model.state(of: $0) == .missing }?.key ?? model.spec.fields.first?.key
        }
    }

    private var fieldList: some View {
        VStack(spacing: 12) {
            ForEach(model.spec.fields) { field in
                FieldCard(model: model, field: field, focused: $focused, onSubmit: advance)
            }
        }
        .padding(.horizontal, 22)
        .padding(.bottom, 4)
    }

    private var header: some View {
        HStack(alignment: .top, spacing: 14) {
            ZStack {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(LinearGradient(colors: [Color(red: 0.36, green: 0.42, blue: 0.98), Color(red: 0.55, green: 0.33, blue: 0.93)],
                                         startPoint: .topLeading, endPoint: .bottomTrailing))
                Image(systemName: "key.horizontal.fill")
                    .font(.system(size: 20, weight: .semibold))
                    .foregroundStyle(.white)
            }
            .frame(width: 44, height: 44)
            .shadow(color: .black.opacity(0.18), radius: 4, y: 2)

            VStack(alignment: .leading, spacing: 5) {
                Text(model.spec.title)
                    .font(.system(size: 16, weight: .semibold))
                if let message = model.spec.message {
                    Text(.init(message))
                        .font(.system(size: 12.5))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                HStack(spacing: 5) {
                    Image(systemName: "lock.shield")
                        .font(.system(size: 10, weight: .semibold))
                    Text(model.spec.destination)
                        .font(.system(size: 11, design: .monospaced))
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                .foregroundStyle(.secondary)
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background(Capsule().fill(.primary.opacity(0.06)))
                .help("Where the values will be written")
            }
            Spacer(minLength: 0)
        }
    }

    private var footer: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 6) {
                Image(systemName: "eye.slash")
                Text("Goes straight to the vault. The agent only learns whether it was saved.")
            }
            .font(.system(size: 11))
            .foregroundStyle(.tertiary)

            HStack {
                if model.pastedFromClipboard {
                    Toggle("Clear clipboard after saving", isOn: $model.clearClipboard)
                        .toggleStyle(.checkbox)
                        .font(.system(size: 12))
                }
                Spacer()
                Button("Cancel") { finish(.cancelled("cancelled")) }
                    .keyboardShortcut(.cancelAction)
                    .controlSize(.large)
                Button {
                    submit()
                } label: {
                    Text(model.spec.fields.count > 1 ? "Save secrets" : "Save secret")
                        .frame(minWidth: 96)
                }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(!model.canSave)
            }
        }
    }

    /// Return in a field: save if everything is ready, else jump to the next field that isn't.
    private func advance() {
        if model.canSave { submit(); return }
        let fields = model.spec.fields
        let start = (fields.firstIndex { $0.key == focused } ?? -1) + 1
        let ordered = fields[start...] + fields[..<start]
        focused = ordered.first { model.state(of: $0) == .missing }?.key ?? focused
    }

    private func submit() {
        guard let outcome = model.save() else { return }
        withAnimation(.easeOut(duration: 0.15)) { model.saved = true }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.7) { finish(outcome) }
    }
}

struct FieldCard: View {
    @Bindable var model: FormModel
    let field: Field
    var focused: FocusState<String?>.Binding
    let onSubmit: () -> Void

    private var state: FieldState { model.state(of: field) }
    private var revealed: Bool { model.revealed.contains(field.key) }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(field.label)
                    .font(.system(size: 13, weight: .semibold))
                if field.label != field.key {
                    Text(field.key)
                        .font(.system(size: 10.5, design: .monospaced))
                        .foregroundStyle(.tertiary)
                }
                Spacer()
                badge
            }

            if field.sharedAvailable {
                Picker("", selection: sharedChoice) {
                    Text("Use shared value").tag(true)
                    Text(field.existing ? "Keep or replace" : "New value for this project").tag(false)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
            }

            if state == .linked {
                HStack(spacing: 6) {
                    Image(systemName: "link")
                    Text("Uses the value saved in shared. Nothing to type.")
                }
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .padding(.vertical, 4)
            } else {
                input
                if model.spec.canShare && !field.sharedAvailable && !model.value(field.key).isEmpty {
                    Toggle("Also save to shared, so other projects can use it", isOn: toSharedBinding)
                        .toggleStyle(.checkbox)
                        .font(.system(size: 11.5))
                }
            }

            let warnings = model.warnings(for: field)
            if !warnings.isEmpty {
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(warnings, id: \.self) { w in
                        Label(w, systemImage: "exclamationmark.triangle")
                            .font(.system(size: 11.5))
                            .foregroundStyle(.orange)
                    }
                    Text("You can still save it.")
                        .font(.system(size: 10.5))
                        .foregroundStyle(.tertiary)
                }
            } else if let help = field.help, state != .linked {
                Text(.init(help))
                    .font(.system(size: 11.5))
                    .foregroundStyle(.secondary)
                    .tint(.accentColor)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(.primary.opacity(0.035)))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(.primary.opacity(0.07), lineWidth: 0.5))
    }

    private var input: some View {
        HStack(spacing: 6) {
            Group {
                if revealed {
                    TextField(placeholder, text: binding)
                } else {
                    SecureField(placeholder, text: binding)
                }
            }
            .textFieldStyle(.plain)
            .font(.system(size: 13, design: .monospaced))
            .focused(focused, equals: field.key)
            .onSubmit(onSubmit)

            iconButton(revealed ? "eye.slash" : "eye", help: revealed ? "Hide" : "Show") {
                if revealed { model.revealed.remove(field.key) } else { model.revealed.insert(field.key) }
            }
            iconButton("doc.on.clipboard", help: "Paste from clipboard") {
                if let s = NSPasteboard.general.string(forType: .string) {
                    model.values[field.key] = s.trimmingCharacters(in: .whitespacesAndNewlines)
                    model.pastedFromClipboard = true
                }
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Color(nsColor: .textBackgroundColor).opacity(0.7)))
        .overlay(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(borderColor, lineWidth: focused.wrappedValue == field.key ? 1.5 : 0.5)
        )
        .animation(.easeOut(duration: 0.12), value: state)
    }

    private var sharedChoice: Binding<Bool> {
        Binding(get: { model.useShared.contains(field.key) },
                set: { if $0 { model.useShared.insert(field.key) } else { model.useShared.remove(field.key) } })
    }

    private var toSharedBinding: Binding<Bool> {
        Binding(get: { model.toShared.contains(field.key) },
                set: { if $0 { model.toShared.insert(field.key) } else { model.toShared.remove(field.key) } })
    }

    private var binding: Binding<String> {
        Binding(get: { model.values[field.key] ?? "" }, set: { model.values[field.key] = $0 })
    }

    private var placeholder: String {
        if field.existing { return "Leave empty to keep the current value" }
        return field.placeholder ?? (field.optional ? "Optional" : "Paste or type")
    }

    private var borderColor: Color {
        switch state {
        case .warned: return .orange
        case .ready: return .green.opacity(0.8)
        default: return focused.wrappedValue == field.key ? .accentColor : .primary.opacity(0.15)
        }
    }

    @ViewBuilder private var badge: some View {
        switch state {
        case .ready: pill("Ready", "checkmark.circle.fill", .green)
        case .kept: pill("Already set", "checkmark.seal", .secondary)
        case .warned: pill("Check it", "exclamationmark.circle.fill", .orange)
        case .linked: pill("Shared", "link", .blue)
        case .optionalEmpty: pill("Optional", "minus.circle", .secondary)
        case .missing: pill("Required", "circle.dashed", .secondary)
        }
    }

    private func pill(_ text: String, _ icon: String, _ color: Color) -> some View {
        Label(text, systemImage: icon)
            .font(.system(size: 10.5, weight: .medium))
            .foregroundStyle(color)
            .labelStyle(.titleAndIcon)
    }

    private func iconButton(_ symbol: String, help: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.secondary)
                .frame(width: 24, height: 22)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(help)
    }
}

struct SavedBadge: View {
    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 44))
                .foregroundStyle(.green)
                .symbolEffect(.bounce, value: true)
            Text("Saved")
                .font(.system(size: 14, weight: .semibold))
        }
        .padding(28)
        .background(RoundedRectangle(cornerRadius: 18, style: .continuous).fill(.thickMaterial))
        .shadow(color: .black.opacity(0.2), radius: 12, y: 4)
        .transition(.scale(scale: 0.9).combined(with: .opacity))
    }
}
