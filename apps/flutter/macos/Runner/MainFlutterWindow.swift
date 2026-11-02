import Cocoa
import FlutterMacOS
import AVFoundation

class MainFlutterWindow: NSWindow {
  var shell: FlutterMethodChannel?
  private var folderPicker: NSOpenPanel?
  private let qrCamera = QrCameraCapture()
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
      if call.method == "pickQrImage" {
        guard let self = self else { result(nil); return }
        self.pickFolder(nil, qrImage: true, result: result)
        return
      }
      if call.method == "qrCapabilities" {
        result(["camera": AVCaptureDevice.default(for: .video) != nil, "import": true])
        return
      }
      if call.method == "startQrCamera" { self?.qrCamera.start(result); return }
      if call.method == "qrCameraFrame" { self?.qrCamera.frame(result); return }
      if call.method == "stopQrCamera" { self?.qrCamera.stop(); result(nil); return }
      if call.method == "takeLink" { result((NSApp.delegate as? AppDelegate)?.takeLink()); return }
      guard call.method == "status", let args = call.arguments as? [String: Any] else { result(FlutterMethodNotImplemented); return }
      (NSApp.delegate as? AppDelegate)?.updateStatus(args)
      result(nil)
    }
    super.awakeFromNib()
  }

  private func pickFolder(_ arguments: [String: Any]?, qrImage: Bool = false, result: @escaping FlutterResult) {
    guard folderPicker == nil else {
      result(FlutterError(code: "picker_busy", message: "A folder picker is already open.", details: nil))
      return
    }
    let picker = NSOpenPanel()
    picker.title = qrImage ? "Choose pairing QR image" : "Choose receiving folder"
    picker.prompt = qrImage ? "Open image" : "Choose folder"
    picker.canChooseDirectories = !qrImage
    picker.canChooseFiles = qrImage
    if qrImage { picker.allowedFileTypes = ["png", "jpg", "jpeg", "webp", "bmp"] }
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
      guard path.hasPrefix("/"), FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory), isDirectory.boolValue != qrImage else {
        result(FlutterError(code: "folder_unavailable", message: "Choose an existing local folder.", details: nil))
        return
      }
      result(path)
    }
  }
}

/// Native camera permission/capture only; Flutter owns the shared scanner page.
private final class QrCameraCapture: NSObject, AVCapturePhotoCaptureDelegate {
  private let queue = DispatchQueue(label: "app.wispnet.qr-camera")
  private var session: AVCaptureSession?
  private var output: AVCapturePhotoOutput?
  private var pending: FlutterResult?
  private var captureId = 0
  private var generation = 0
  private var photoId: Int64?

  func start(_ result: @escaping FlutterResult) {
    generation += 1
    let current = generation
    let begin = { [weak self] in
      guard let self = self else { result(nil); return }
      guard self.generation == current else {
        result(FlutterError(code: "camera_closed", message: "Camera closed.", details: nil))
        return
      }
      self.queue.async {
        do {
          guard let camera = AVCaptureDevice.default(for: .video) else {
            throw NSError(domain: "camera", code: 1)
          }
          let session = AVCaptureSession()
          session.sessionPreset = .vga640x480
          let input = try AVCaptureDeviceInput(device: camera)
          let output = AVCapturePhotoOutput()
          guard session.canAddInput(input), session.canAddOutput(output) else {
            throw NSError(domain: "camera", code: 2)
          }
          session.addInput(input)
          session.addOutput(output)
          session.startRunning()
          DispatchQueue.main.async {
            guard self.generation == current else {
              self.queue.async { session.stopRunning() }
              result(FlutterError(code: "camera_closed", message: "Camera closed.", details: nil))
              return
            }
            self.session = session
            self.output = output
            result(nil)
          }
        } catch {
          DispatchQueue.main.async {
            result(FlutterError(code: "camera_unavailable", message: "Camera unavailable. Check camera access and try again.", details: nil))
          }
        }
      }
    }
    switch AVCaptureDevice.authorizationStatus(for: .video) {
    case .authorized: begin()
    case .notDetermined:
      AVCaptureDevice.requestAccess(for: .video) { allowed in
        DispatchQueue.main.async {
          if allowed { begin() }
          else { result(FlutterError(code: "camera_denied", message: "Allow camera access in System Settings or import a QR image.", details: nil)) }
        }
      }
    default:
      result(FlutterError(code: "camera_denied", message: "Allow camera access in System Settings or import a QR image.", details: nil))
    }
  }

  func frame(_ result: @escaping FlutterResult) {
    guard pending == nil, let output = output, session?.isRunning == true else {
      result(FlutterError(code: "camera_unavailable", message: "Camera unavailable. Try opening it again.", details: nil))
      return
    }
    pending = result
    captureId += 1
    let current = captureId
    let settings = AVCapturePhotoSettings()
    photoId = settings.uniqueID
    output.capturePhoto(with: settings, delegate: self)
    DispatchQueue.main.asyncAfter(deadline: .now() + 6) { [weak self] in
      guard let self = self, self.captureId == current, let pending = self.pending else { return }
      self.pending = nil
      pending(FlutterError(code: "camera_timeout", message: "No camera image received. Try again.", details: nil))
    }
  }

  func photoOutput(_ output: AVCapturePhotoOutput, didFinishProcessingPhoto photo: AVCapturePhoto, error: Error?) {
    DispatchQueue.main.async { [weak self] in
      guard let self = self, self.photoId == photo.resolvedSettings.uniqueID, let pending = self.pending else { return }
      self.pending = nil
      guard error == nil, let data = photo.fileDataRepresentation(), data.count <= 4 * 1024 * 1024 else {
        pending(FlutterError(code: "camera_unavailable", message: "Cannot read the camera image. Try again.", details: nil))
        return
      }
      pending(FlutterStandardTypedData(bytes: data))
    }
  }

  func stop() {
    generation += 1
    captureId += 1
    let pending = self.pending
    self.pending = nil
    photoId = nil
    pending?(FlutterError(code: "camera_closed", message: "Camera closed.", details: nil))
    let session = self.session
    self.session = nil
    output = nil
    queue.async { session?.stopRunning() }
  }
}
