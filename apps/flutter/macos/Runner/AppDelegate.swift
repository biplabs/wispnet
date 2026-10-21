import Cocoa
import FlutterMacOS

@main
class AppDelegate: FlutterAppDelegate {
  private var item: NSStatusItem?
  private let status = NSMenuItem(title: "Wispnet", action: nil, keyEquivalent: "")
  private let toggle = NSMenuItem(title: "Go online", action: #selector(toggleNode), keyEquivalent: "")
  override func applicationDidFinishLaunching(_ notification: Notification) {
    item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    item?.button?.title = "W"
    let menu = NSMenu()
    menu.addItem(status)
    menu.addItem(withTitle: "Show Wispnet", action: #selector(reveal), keyEquivalent: "")
    menu.addItem(toggle)
    menu.addItem(withTitle: "Quit Wispnet (node keeps running)", action: #selector(quit), keyEquivalent: "q")
    for entry in menu.items { entry.target = self }
    item?.menu = menu
    if CommandLine.arguments.contains("--background") || CommandLine.arguments.contains("--tray") {
      mainFlutterWindow?.orderOut(nil)
      NSApp.setActivationPolicy(.accessory)
    }
  }
  func updateStatus(_ args: [String: Any]) {
    status.title = args["text"] as? String ?? "Wispnet"
    item?.button?.toolTip = status.title
    toggle.title = args["online"] as? Bool == true ? "Go offline" : "Go online"
  }
  @objc private func toggleNode() {
    (mainFlutterWindow as? MainFlutterWindow)?.shell?.invokeMethod("toggle", arguments: nil) { result in
      if result is FlutterError { self.reveal() }
    }
  }
  @objc private func reveal() {
    NSApp.setActivationPolicy(.regular)
    mainFlutterWindow?.makeKeyAndOrderFront(nil)
    NSApp.activate(ignoringOtherApps: true)
  }
  @objc private func quit() { NSApp.terminate(nil) }
  override func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
    reveal(); return true
  }
  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool { true }
}
