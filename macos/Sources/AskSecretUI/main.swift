import AppKit
import SwiftUI

// The popup half of ask-secret. It reads a JSON spec on stdin and writes the answers
// as JSON on stdout to the `ask-secret` process that started it. It stores nothing.

let spec: Spec
do {
    spec = try JSONDecoder().decode(Spec.self, from: FileHandle.standardInput.readDataToEndOfFile())
} catch {
    FileHandle.standardError.write(Data("ask-secret-ui: bad spec on stdin: \(error)\n".utf8))
    exit(2)
}

func emit(_ object: [String: Any]) {
    let data = (try? JSONSerialization.data(withJSONObject: object)) ?? Data()
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data("\n".utf8))
}

final class Controller: NSObject, NSApplicationDelegate, NSWindowDelegate {
    let model: FormModel
    var window: NSPanel!
    private var finished = false

    init(spec: Spec) {
        self.model = FormModel(spec: spec)
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let root = SecretForm(model: model) { [weak self] outcome in self?.finish(outcome) }
        let hosting = NSHostingController(rootView: root)
        // Track SwiftUI's preferred size, so the panel grows when a warning appears.
        hosting.sizingOptions = [.preferredContentSize]
        let size = hosting.sizeThatFits(in: NSSize(width: 500, height: 10_000))

        window = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 500, height: 300),
                         styleMask: [.titled, .closable, .fullSizeContentView],
                         backing: .buffered, defer: false)
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.isMovableByWindowBackground = true
        window.level = .floating
        window.hidesOnDeactivate = false
        for kind in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
            window.standardWindowButton(kind)?.isHidden = true
        }
        window.contentViewController = hosting
        window.setContentSize(size)
        window.delegate = self
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

        DispatchQueue.main.asyncAfter(deadline: .now() + model.spec.timeout) { [weak self] in
            self?.finish(.cancelled("timed out"))
        }
    }

    func windowWillClose(_ notification: Notification) {
        finish(.cancelled(""))
    }

    func finish(_ outcome: Outcome) {
        guard !finished else { return }
        finished = true
        switch outcome {
        case .saved(let answers):
            emit(["status": "saved", "fields": answers])
            exit(0)
        case .cancelled(let why):
            emit(["status": "cancelled", "reason": why])
            exit(1)
        }
    }
}

let app = NSApplication.shared
let controller = Controller(spec: spec)
app.delegate = controller
app.setActivationPolicy(.accessory)
app.run()
