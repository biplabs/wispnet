import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  var shell: FlutterMethodChannel?
  private var folderPicker: NSOpenPanel?
  override func awakeFromNib() {
    let controller = FlutterViewController()
    let windowFrame = frame
    contentViewController = controller
    setFrame(windowFrame, display: true)
    minSize = NSSize(width: 360, height: 440)
    title = "Wispnet"
    RegisterGeneratedPlugins(registry: controller)
    shell = FlutterMethodChannel(name: "app.wispnet/desktop", binaryMessenger: controller.engine.binaryMessenger)
    shell?.setMethodCallHandler { [weak self] call, result in
      if call.method == "pickFolder" {
        guard let self = self else {
          result(FlutterError(code: "window_unavailable", message: "The Wispnet window is unavailable.", details: nil))
          return
        }
        self.pickFolder(call.arguments as? [String: Any], result: result)
        return
      }
      if call.method == "takeLink" { result((NSApp.delegate as? AppDelegate)?.takeLink()); return }
      guard call.method == "status", let args = call.arguments as? [String: Any] else { result(FlutterMethodNotImplemented); return }
      (NSApp.delegate as? AppDelegate)?.updateStatus(args)
      result(nil)
    }
    super.awakeFromNib()
  }

  private func pickFolder(_ arguments: [String: Any]?, result: @escaping FlutterResult) {
    guard folderPicker == nil else {
      result(FlutterError(code: "picker_busy", message: "A folder picker is already open.", details: nil))
      return
    }
    let picker = NSOpenPanel()
    picker.title = "Choose receiving folder"
    picker.prompt = "Choose folder"
    picker.canChooseDirectories = true
    picker.canChooseFiles = false
    picker.allowsMultipleSelection = false
    picker.canCreateDirectories = true
    picker.resolvesAliases = true
    if let initialPath = arguments?["initialPath"] as? String, initialPath.hasPrefix("/") {
      var isDirectory: ObjCBool = false
      if FileManager.default.fileExists(atPath: initialPath, isDirectory: &isDirectory), isDirectory.boolValue {
        picker.directoryURL = URL(fileURLWithPath: initialPath, isDirectory: true)
      }
    }
    folderPicker = picker
    picker.beginSheetModal(for: self) { [weak self] response in
      self?.folderPicker = nil
      guard response == .OK else { result(nil); return }
      guard let url = picker.url else {
        result(FlutterError(code: "folder_unavailable", message: "Choose an existing local folder.", details: nil))
        return
      }
      let path = url.resolvingSymlinksInPath().standardizedFileURL.path
      var isDirectory: ObjCBool = false
      guard path.hasPrefix("/"), FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory), isDirectory.boolValue else {
        result(FlutterError(code: "folder_unavailable", message: "Choose an existing local folder.", details: nil))
        return
      }
      result(path)
    }
  }
}
