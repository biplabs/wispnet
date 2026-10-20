import AppKit
import Darwin
import SwiftUI
import WispnetCore

@MainActor final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private let model = Model()
    private var window: NSWindow!
    private var statusItem: NSStatusItem!
    private let menu = NSMenu()
    private var instanceFD: Int32 = -1
    private var revealObserver: NSObjectProtocol?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let path = URL(fileURLWithPath: model.api.socketPath).deletingLastPathComponent()
            .appendingPathComponent("macos-desktop.lock").path
        instanceFD = Darwin.open(path, O_RDWR | O_CREAT | O_NOFOLLOW | O_NONBLOCK, 0o600)
        var metadata = stat()
        guard instanceFD >= 0, fstat(instanceFD, &metadata) == 0,
              metadata.st_mode & S_IFMT == S_IFREG, metadata.st_uid == geteuid(),
              metadata.st_mode & 0o077 == 0 else {
            let alert = NSAlert()
            alert.messageText = "Cannot acquire the private Wispnet app lock"
            alert.informativeText = "Check the node state directory and reinstall Wispnet."
            alert.runModal()
            NSApp.terminate(nil)
            return
        }
        if flock(instanceFD, LOCK_EX | LOCK_NB) != 0 {
            DistributedNotificationCenter.default().postNotificationName(
                Notification.Name("app.wispnet.reveal"), object: nil, deliverImmediately: true)
            NSApp.terminate(nil)
            return
        }
        revealObserver = DistributedNotificationCenter.default().addObserver(
            forName: Notification.Name("app.wispnet.reveal"), object: nil, queue: .main
        ) { [weak self] _ in Task { @MainActor in self?.showWindow() } }
        let root = ContentView(model: model)
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1040, height: 720),
                          styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = "Wispnet"
        window.setFrameAutosaveName("WispnetMainWindow")
        window.contentView = NSHostingView(rootView: root)
        window.isReleasedWhenClosed = false
        window.delegate = self
        window.center()
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        statusItem.button?.title = "W"
        statusItem.button?.font = .systemFont(ofSize: 14, weight: .semibold)
        statusItem.button?.setAccessibilityLabel("Wispnet")
        statusItem.menu = menu
        model.onStatus = { [weak self] in self?.updateMenu() }
        installMainMenu()
        updateMenu()
        model.start()
        if !CommandLine.arguments.contains("--background") { showWindow() }
    }
    private func item(_ title: String, _ action: Selector? = nil, key: String = "") -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.target = self
        return item
    }
    private func installMainMenu() {
        let main = NSMenu()
        let app = NSMenuItem()
        let appMenu = NSMenu()
        appMenu.addItem(item("About Wispnet", #selector(about)))
        appMenu.addItem(.separator())
        appMenu.addItem(item("Hide Wispnet", #selector(hideApp), key: "h"))
        appMenu.addItem(item("Quit Wispnet", #selector(quit), key: "q"))
        app.submenu = appMenu
        main.addItem(app)
        let edit = NSMenuItem(title: "Edit", action: nil, keyEquivalent: "")
        let editMenu = NSMenu(title: "Edit")
        for (title, action, key) in [("Cut", "cut:", "x"), ("Copy", "copy:", "c"), ("Paste", "paste:", "v"), ("Select All", "selectAll:", "a")] {
            editMenu.addItem(NSMenuItem(title: title, action: Selector(action), keyEquivalent: key))
        }
        edit.submenu = editMenu; main.addItem(edit)
        let view = NSMenuItem(title: "Window", action: nil, keyEquivalent: "")
        let windowMenu = NSMenu(title: "Window")
        windowMenu.addItem(item("Show Wispnet", #selector(showWindow), key: "0"))
        view.submenu = windowMenu; main.addItem(view)
        NSApp.mainMenu = main
    }
    private func updateMenu() {
        menu.removeAllItems()
        menu.addItem(item(model.online ? "Wispnet · Online" : "Wispnet · Offline"))
        if let status = model.status {
            menu.addItem(item("\(status.activeClients) active · \(status.host.clients.count) approved"))
            menu.addItem(item("\(status.pending.count) pairing requests · Relay: \(status.relay)"))
            statusItem.button?.toolTip = "Wispnet · \(status.activeClients) active devices · \(status.pending.count) requests"
        } else { statusItem.button?.toolTip = "Wispnet · node offline" }
        menu.addItem(.separator())
        menu.addItem(item("Show Wispnet", #selector(showWindow)))
        let toggle = item(model.online ? "Go offline" : "Go online", #selector(toggleOnline))
        toggle.isEnabled = !model.busy; menu.addItem(toggle)
        menu.addItem(.separator())
        menu.addItem(item("Quit Wispnet (node keeps running)", #selector(quit), key: "q"))
        menu.autoenablesItems = false
        // Status-only rows stay noninteractive.
        for row in menu.items where row.action == nil { row.isEnabled = false }
    }
    @objc func showWindow() {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
    }
    @objc private func toggleOnline() { model.setOnline(!model.online); updateMenu() }
    @objc private func hideApp() { NSApp.hide(nil) }
    @objc private func about() { NSApp.orderFrontStandardAboutPanel(nil) }
    @objc private func quit() { NSApp.terminate(nil) }
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        sender.orderOut(nil)
        NSApp.setActivationPolicy(.accessory)
        return false
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        showWindow(); return true
    }
}

@main @MainActor struct WispnetMain {
    static func main() {
        // Dependency-free IPC probe also exercises the exact transport used by the UI.
        if let index = CommandLine.arguments.firstIndex(of: "--request"), CommandLine.arguments.count > index + 1 {
            do {
                let input = FileHandle.standardInput.readData(ofLength: 16385)
                guard input.count <= 16384, let payload = try JSONSerialization.jsonObject(with: input) as? [String: Any] else {
                    throw NodeError("Expected one JSON object of at most 16 KiB on stdin.")
                }
                var result = try NodeAPI(socketPath: CommandLine.arguments[index + 1]).request(payload)
                result.append(10)
                FileHandle.standardOutput.write(result)
            } catch {
                FileHandle.standardError.write(Data((error.localizedDescription + "\n").utf8))
                exit(1)
            }
        } else {
            let application = NSApplication.shared
            let delegate = AppDelegate()
            application.delegate = delegate
            application.setActivationPolicy(CommandLine.arguments.contains("--background") ? .accessory : .regular)
            application.run()
        }
    }
}
