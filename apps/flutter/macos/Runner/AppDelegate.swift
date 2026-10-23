import Cocoa
import FlutterMacOS

@main
class AppDelegate: FlutterAppDelegate {
  private var links: [String] = []
  func takeLink() -> String? { links.isEmpty ? nil : links.removeFirst() }
  override func application(_ application: NSApplication, open urls: [URL]) {
    for url in urls where url.scheme == "wispnet" && url.absoluteString.count <= 256 && links.count < 16 { links.append(url.absoluteString) }
    reveal()
  }
  private var item: NSStatusItem?
  private let status = NSMenuItem(title: "Wispnet", action: nil, keyEquivalent: "")
  private let menu = NSMenu()
  override func applicationDidFinishLaunching(_ notification: Notification) {
    item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    item?.button?.title = "W"
    menu.addItem(status)
    menu.addItem(withTitle: "Exit", action: #selector(quit), keyEquivalent: "q")
    for entry in menu.items { entry.target = self }
    item?.button?.target = self
    item?.button?.action = #selector(trayClicked)
    item?.button?.sendAction(on: [.leftMouseUp, .rightMouseUp])
    if CommandLine.arguments.contains("--background") || CommandLine.arguments.contains("--tray") {
      mainFlutterWindow?.orderOut(nil)
      NSApp.setActivationPolicy(.accessory)
    }
  }
  func updateStatus(_ args: [String: Any]) {
    status.title = args["text"] as? String ?? "Wispnet"
    item?.button?.toolTip = status.title
  }
  @objc private func trayClicked() {
    if NSApp.currentEvent?.type == .rightMouseUp, let button = item?.button {
      menu.popUp(positioning: nil, at: NSPoint(x: 0, y: button.bounds.minY), in: button)
    } else {
      reveal()
    }
  }
  @objc private func reveal() {
    NSApp.setActivationPolicy(.regular)
    mainFlutterWindow?.makeKeyAndOrderFront(nil)
    NSApp.activate(ignoringOtherApps: true)
  }
  @objc private func quit() {
    guard let shell = (mainFlutterWindow as? MainFlutterWindow)?.shell else { return }
    shell.invokeMethod("exit", arguments: nil) { result in
      if result is FlutterError || (result as? NSObject) == FlutterMethodNotImplemented {
        self.reveal()
        let alert = NSAlert()
        alert.messageText = "Could not stop Wispnet. Please try again."
        alert.runModal()
      } else {
        NSApp.terminate(nil)
      }
    }
  }
  override func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
    reveal(); return true
  }
  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool { true }
}
