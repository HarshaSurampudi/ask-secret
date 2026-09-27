import AppKit
import SwiftUI

enum Outcome {
    case saved([String: String])  // key -> "saved" | "kept" | "skipped"
    case cancelled(String)
}

@Observable
final class FormModel {
    let spec: Spec
    let existing: Set<String>
    var values: [String: String]
    var revealed: Set<String> = []
    var pastedFromClipboard = false
    var clearClipboard = true
    var saving = false
    var saved = false
    var error: String?

    init(spec: Spec) {
        self.spec = spec
        self.existing = Store.existingKeys(in: spec.destination, keys: spec.fields.map(\.key))
        self.values = Dictionary(uniqueKeysWithValues: spec.fields.map { ($0.key, "") })
    }

    func value(_ key: String) -> String {
        (values[key] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func state(of field: Field) -> FieldState {
        let v = value(field.key)
        if v.isEmpty {
            if existing.contains(field.key) { return .kept }
            return field.optional ? .optionalEmpty : .missing
        }
        return field.isValid(v) ? .ready : .invalid
    }

    var canSave: Bool {
        !saving && spec.fields.allSatisfy { [.ready, .kept, .optionalEmpty].contains(state(of: $0)) }
            && spec.fields.contains { state(of: $0) == .ready }
    }

    func save() -> Outcome? {
        guard canSave else { return nil }
        saving = true
        var toWrite: [String: String] = [:]
        var report: [String: String] = [:]
        for f in spec.fields {
            switch state(of: f) {
            case .ready: toWrite[f.key] = value(f.key); report[f.key] = "saved"
            case .kept: report[f.key] = "kept (already set)"
            default: report[f.key] = "skipped"
            }
        }
        do {
            try Store.save(toWrite, to: spec.destination)
        } catch {
            self.error = error.localizedDescription
            saving = false
            return nil
        }
        if pastedFromClipboard && clearClipboard { NSPasteboard.general.clearContents() }
        values = values.mapValues { _ in "" }
        return .saved(report)
    }
}

enum FieldState { case missing, invalid, ready, kept, optionalEmpty }

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

            if let error = model.error {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .font(.system(size: 12))
                    .foregroundStyle(.red)
                    .padding(.horizontal, 22)
                    .padding(.top, 10)
            }

            footer
                .padding(.horizontal, 22)
                .padding(.vertical, 16)
        }
        .frame(width: 500)
        .background(.regularMaterial)
        .overlay(alignment: .center) { if model.saved { SavedBadge() } }
        .onAppear {
            focused = model.spec.fields.first { !model.existing.contains($0.key) }?.key ?? model.spec.fields.first?.key
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
                    Image(systemName: destinationIcon)
                        .font(.system(size: 10, weight: .semibold))
                    Text(model.spec.destination.display)
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

    private var destinationIcon: String {
        if case .keychain = model.spec.destination { return "lock.shield" }
        return "doc.badge.gearshape"
    }

    private var footer: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 6) {
                Image(systemName: "eye.slash")
                Text("Goes straight to the file. Claude only learns whether it was saved.")
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
        focused = ordered.first { [.missing, .invalid].contains(model.state(of: $0)) }?.key ?? focused
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

            if state == .invalid {
                Label(field.patternHint ?? "This doesn't look like the expected format.", systemImage: "exclamationmark.circle")
                    .font(.system(size: 11.5))
                    .foregroundStyle(.orange)
            } else if let help = field.help {
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

    private var binding: Binding<String> {
        Binding(get: { model.values[field.key] ?? "" }, set: { model.values[field.key] = $0 })
    }

    private var placeholder: String {
        if model.existing.contains(field.key) { return "Leave empty to keep the current value" }
        return field.placeholder ?? (field.optional ? "Optional" : "Paste or type")
    }

    private var borderColor: Color {
        switch state {
        case .invalid: return .orange
        case .ready: return .green.opacity(0.8)
        default: return focused.wrappedValue == field.key ? .accentColor : .primary.opacity(0.15)
        }
    }

    @ViewBuilder private var badge: some View {
        switch state {
        case .ready: pill("Ready", "checkmark.circle.fill", .green)
        case .kept: pill("Already set", "checkmark.seal", .secondary)
        case .invalid: pill("Check format", "exclamationmark.circle.fill", .orange)
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
