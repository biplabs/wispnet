import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  var shell: FlutterMethodChannel?
  override func awakeFromNib() {
    let controller = FlutterViewController()
    let windowFrame = frame
    contentViewController = controller
    setFrame(windowFrame, display: true)
    minSize = NSSize(width: 360, height: 440)
    title = "Wispnet"
    RegisterGeneratedPlugins(registry: controller)
    shell = FlutterMethodChannel(name: "app.wispnet/desktop", binaryMessenger: controller.engine.binaryMessenger)
    shell?.setMethodCallHandler { call, result in
      guard call.method == "status", let args = call.arguments as? [String: Any] else { result(FlutterMethodNotImplemented); return }
      (NSApp.delegate as? AppDelegate)?.updateStatus(args)
      result(nil)
    }
    super.awakeFromNib()
  }
}
