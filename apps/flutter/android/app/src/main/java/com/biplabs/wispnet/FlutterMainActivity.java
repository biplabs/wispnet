package com.biplabs.wispnet;

import android.Manifest;
import android.app.Activity;
import android.content.*;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Bundle;
import android.os.CancellationSignal;
import androidx.annotation.NonNull;
import io.flutter.embedding.android.FlutterActivity;
import io.flutter.embedding.engine.FlutterEngine;
import io.flutter.plugin.common.MethodChannel;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.*;
import org.json.*;

/** Android lifecycle/storage adapter. Flutter never handles the protected key. */
public final class FlutterMainActivity extends FlutterActivity {
 private final ExecutorService work = Executors.newFixedThreadPool(4);
 private final ExecutorService state = Executors.newSingleThreadExecutor();
 private NetworkBridge network;
 private MethodChannel.Result qrResult;
 private MethodChannel.Result fileResult;
 private MethodChannel.Result folderResult;
 private AndroidQrCamera qrCamera;
 private MethodChannel.Result qrCameraPermission;
 private boolean resumed;
 private boolean downloadFolder;
 private boolean shareFolder;
 private CancellationSignal fileCancellation;
 private String invitation, invitationError, serviceLink;
 private final java.util.ArrayDeque<Intent> sharedFiles = new java.util.ArrayDeque<>();

 @Override public void onCreate(Bundle state) {
  super.onCreate(state);
  network = new NetworkBridge(this);
  qrCamera = new AndroidQrCamera(this);
  if (state != null) {
   java.util.ArrayList<Intent> pending = state.getParcelableArrayList("wispnet.shares", Intent.class);
   if (pending != null) sharedFiles.addAll(pending);
  }
  receiveInvitation(getIntent());
  if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED)
   requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 101);
 }
 @Override public void configureFlutterEngine(@NonNull FlutterEngine engine) {
  super.configureFlutterEngine(engine);
  engine.getPlatformViewsController().getRegistry().registerViewFactory(
   "app.wispnet/terminal", new com.biplabs.wispnet.shell.FlutterTerminalViewFactory(engine.getDartExecutor().getBinaryMessenger()));
  new MethodChannel(engine.getDartExecutor().getBinaryMessenger(), "app.wispnet/ui").setMethodCallHandler((call, result) -> {
   final JSONObject arguments;
   try { arguments = new JSONObject(call.arguments == null ? "{}" : (String)call.arguments); }
   catch (Exception invalid) { result.error("invalid_request", "Invalid UI request", null); return; }
   if ("takeShare".equals(call.method)) {
    Intent shared = sharedFiles.pollFirst();
    if (shared == null) { result.success("{}"); return; }
    work.execute(() -> {
     try {
      JSONObject reply = AndroidFilePicker.selection(this, shared, new CancellationSignal());
      if (reply.getJSONArray("files").length() == 0) throw new Exception("No readable files were shared.");
      runOnUiThread(() -> result.success(reply.toString()));
     } catch (Exception failure) {
      runOnUiThread(() -> result.error("share_invalid", "Cannot read shared files. Share them again: " + failure.getMessage(), null));
     }
    });
    return;
   }
   if ("takeLink".equals(call.method)) {
    JSONObject reply = new JSONObject();
    try { if (serviceLink != null) reply.put("link", serviceLink); serviceLink = null; result.success(reply.toString()); }
    catch (JSONException failure) { result.error("invalid_link", failure.getMessage(), null); }
    return;
   }
   if ("takeInvitation".equals(call.method)) {
    JSONObject reply = new JSONObject();
    try { if (invitation != null) reply.put("code", invitation); if (invitationError != null) reply.put("error", invitationError); invitation = null; invitationError = null; result.success(reply.toString()); }
    catch (JSONException failure) { result.error("invitation_invalid", failure.getMessage(), null); }
    return;
   }
   if ("qrCapabilities".equals(call.method)) {
    try { result.success(new JSONObject().put("camera", getPackageManager().hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)).put("import", true).toString()); }
    catch (JSONException failure) { result.error("qr_unavailable", "Cannot inspect camera support", null); }
    return;
   }
   if ("startQrCamera".equals(call.method)) { startQrCamera(result); return; }
   if ("qrCameraFrame".equals(call.method)) {
    qrCamera.frame((bytes, error) -> cameraReply(result, bytes, error)); return;
   }
   if ("stopQrCamera".equals(call.method)) {
    stopQrCamera(); result.success("{}"); return;
   }
   if ("importQr".equals(call.method)) { importQr(result); return; }
   if ("pickReceivingFolder".equals(call.method) || "pickDownloadFolder".equals(call.method) || "pickShareFolder".equals(call.method)) {
    if (folderResult != null || fileResult != null || qrResult != null) { result.error("picker_busy", "A picker is already in progress", null); return; }
    folderResult = result;
    downloadFolder = "pickDownloadFolder".equals(call.method);
    shareFolder = "pickShareFolder".equals(call.method);
    try { startActivityForResult(new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
      .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | (shareFolder ? 0 : Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION | Intent.FLAG_GRANT_PREFIX_URI_PERMISSION), 105); }
    catch (Exception failure) { folderResult = null; result.error("folder_unavailable", "Android's folder picker is unavailable", null); }
    return;
   }
   if ("pickFiles".equals(call.method)) { pickFiles(result); return; }
   if (call.method.startsWith("nativeTerminal") || "saveTerminalLayout".equals(call.method)) {
    try { result.success(com.biplabs.wispnet.shell.FlutterTerminalBridge.dispatch(this, call.method, arguments).toString()); }
    catch (Exception failure) { result.error("terminal_failed", failure.getMessage(), null); }
    return;
   }
   if ("closeShell".equals(call.method)) {
    String host = arguments.optString("host", "");
    if (!host.isEmpty()) com.biplabs.wispnet.shell.ShellWorkspace.INSTANCE.closeHost(this, host, true);
    else com.biplabs.wispnet.shell.ShellWorkspace.INSTANCE.closeAll(this, true);
    result.success("{}");
    return;
   }
   if ("setAccess".equals(call.method)) {
    if (arguments.optBoolean("enabled")) {
     try { startForegroundService(new Intent(this, AccessService.class)); }
     catch (Exception failure) { result.error("access_failed", failure.getMessage(), null); return; }
    } else {
     com.biplabs.wispnet.shell.ShellWorkspace.INSTANCE.closeAll(this, true);
    }
   }
   if ("launch".equals(call.method)) {
    try {
     Uri url = Uri.parse(arguments.getString("url"));
     if (!"http".equals(url.getScheme()) && !"https".equals(url.getScheme())) throw new Exception("Invalid service URL");
     startActivity(new Intent(Intent.ACTION_VIEW, url).addCategory(Intent.CATEGORY_BROWSABLE)); result.success("{}");
    } catch (Exception failure) { result.error("launch_failed", failure.getMessage(), null); }
    return;
   }
   ExecutorService executor = java.util.Set.of("snapshot", "settings", "saveOrder", "preview").contains(call.method) ? state : work;
   executor.execute(() -> {
    try {
     JSONObject reply = network.request(call.method, arguments);
     if ("snapshot".equals(call.method)) {
      reply.put("ui_pid", android.os.Process.myPid());
      reply.put("terminal_layout", com.biplabs.wispnet.shell.FlutterTerminalBridge.savedLayout(this));
      JSONObject shell = com.biplabs.wispnet.shell.ShellWorkspace.INSTANCE.snapshotJson();
      if (shell != null) reply.put("shell", shell);
      JSONArray shells = com.biplabs.wispnet.shell.ShellWorkspace.INSTANCE.snapshotsJson();
      if (shells.length() > 0) reply.put("shells", shells);
     }
     runOnUiThread(() -> {
      try {
       if ("removeHost".equals(call.method)) {
        com.biplabs.wispnet.shell.ShellWorkspace.INSTANCE.closeHost(this, arguments.optString("host"), true);
       }
       result.success(reply.toString());
      } catch (Exception failure) { result.error("terminal_failed", failure.getMessage(), null); }
     });
    } catch (Exception failure) {
     String code = failure instanceof NetworkBridge.Failure ? ((NetworkBridge.Failure)failure).code : "operation_failed";
     runOnUiThread(() -> result.error(code, failure.getMessage(), null));
    }
   });
  });
 }
 private void cameraReply(MethodChannel.Result result, byte[] bytes, Exception error) {
  if (error != null) { runOnUiThread(() -> result.error("camera_unavailable", error.getMessage(), null)); return; }
  try {
   String reply = bytes == null ? "{}" : new JSONObject().put("image_b64", android.util.Base64.encodeToString(bytes, android.util.Base64.NO_WRAP)).toString();
   runOnUiThread(() -> result.success(reply));
  } catch (Exception failure) { runOnUiThread(() -> result.error("camera_unavailable", "Cannot read camera image. Try again.", null)); }
 }
 private void startQrCamera(MethodChannel.Result result) {
  if (qrCameraPermission != null || qrResult != null || fileResult != null || folderResult != null) {
   result.error("camera_busy", "A camera or picker is already open.", null); return;
  }
  if (checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
   qrCameraPermission = result;
   requestPermissions(new String[]{Manifest.permission.CAMERA}, 106);
   return;
  }
  if (!resumed) { result.error("camera_unavailable", "Return to Wispnet to use the camera.", null); return; }
  qrCamera.start((bytes, error) -> cameraReply(result, bytes, error));
 }
 private void stopQrCamera() {
  if (qrCameraPermission != null) {
   MethodChannel.Result pending = qrCameraPermission; qrCameraPermission = null;
   pending.error("camera_closed", "Camera closed.", null);
  }
  if (qrCamera != null) qrCamera.close();
 }
 @Override public void onRequestPermissionsResult(int request, @NonNull String[] permissions, @NonNull int[] grants) {
  super.onRequestPermissionsResult(request, permissions, grants);
  if (request != 106 || qrCameraPermission == null) return;
  if (grants.length == 0 || grants[0] != PackageManager.PERMISSION_GRANTED) {
   MethodChannel.Result pending = qrCameraPermission; qrCameraPermission = null;
   pending.error("camera_denied", "Allow camera access or import a QR image.", null);
  } else if (resumed) {
   MethodChannel.Result pending = qrCameraPermission; qrCameraPermission = null;
   startQrCamera(pending);
  }
 }
 @Override protected void onResume() {
  super.onResume(); resumed = true;
  if (qrCameraPermission != null && checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
   MethodChannel.Result pending = qrCameraPermission; qrCameraPermission = null;
   startQrCamera(pending);
  }
 }
 @Override protected void onPause() {
  resumed = false;
  if (qrCamera != null) qrCamera.close();
  super.onPause();
 }
 private void importQr(MethodChannel.Result result) {
  if (folderResult != null || qrResult != null || fileResult != null) { result.error("qr_busy", "A picker is already in progress", null); return; }
  try {
   qrResult = result;
   startActivityForResult(new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("image/*").addCategory(Intent.CATEGORY_OPENABLE), 103);
  } catch (Exception failure) { qrResult = null; result.error("qr_unavailable", failure.getMessage(), null); }
 }
 private void pickFiles(MethodChannel.Result result) {
  if (folderResult != null || fileResult != null || qrResult != null) { result.error("files_busy", "A picker is already in progress", null); return; }
  try {
   fileResult = result;
   fileCancellation = new CancellationSignal();
   startActivityForResult(AndroidFilePicker.intent(), 104);
  } catch (Exception failure) {
   fileResult = null; fileCancellation = null;
   result.error("files_unavailable", "Android's file picker is unavailable.", null);
  }
 }
 @Override protected void onActivityResult(int request, int resultCode, Intent data) {
  super.onActivityResult(request, resultCode, data);
  if (request == 105 && folderResult != null) {
   MethodChannel.Result result = folderResult;
   if (resultCode != Activity.RESULT_OK || data == null || data.getData() == null) {
    folderResult = null; result.success("{}"); return;
   }
   Uri tree = data.getData();
   int flags = data.getFlags() & (Intent.FLAG_GRANT_READ_URI_PERMISSION | (shareFolder ? 0 : Intent.FLAG_GRANT_WRITE_URI_PERMISSION));
   String prepareFolder = shareFolder ? "prepareShareFolder" : downloadFolder ? "prepareDownloadFolder" : "prepareReceivingFolder";
   work.execute(() -> {
    try {
     getContentResolver().takePersistableUriPermission(tree, flags);
     String reply = network.request(prepareFolder, new JSONObject().put("uri", tree.toString())).toString();
     runOnUiThread(() -> { if (folderResult != result) return; folderResult = null; result.success(reply); });
    } catch (Exception failure) {
     runOnUiThread(() -> { if (folderResult != result) return; folderResult = null; result.error("folder_invalid", "Cannot use this folder. Choose a folder and allow access.", null); });
    }
   });
   return;
  }
  if (request == 104 && fileResult != null) {
   MethodChannel.Result result = fileResult;
   if (resultCode != Activity.RESULT_OK || data == null) {
    fileResult = null; fileCancellation = null; result.success("{\"files\":[]}"); return;
   }
   CancellationSignal cancellation = fileCancellation;
   work.execute(() -> {
    try {
     String reply = AndroidFilePicker.selection(this, data, cancellation).toString();
     runOnUiThread(() -> {
      if (fileResult != result) return;
      fileResult = null; fileCancellation = null; result.success(reply);
     });
    } catch (Exception failure) {
     runOnUiThread(() -> {
      if (fileResult != result) return;
      fileResult = null; fileCancellation = null;
      result.error("files_invalid", failure.getMessage() == null ? "Cannot read these files. Select them again." : failure.getMessage(), null);
     });
    }
   });
   return;
  }
  if (request != 103 || qrResult == null) return;
  MethodChannel.Result result = qrResult; qrResult = null;
  if (resultCode != Activity.RESULT_OK || data == null) { result.success("{}"); return; }
  work.execute(() -> {
   try {
    Uri uri = data.getData();
    if (uri == null || !"content".equals(uri.getScheme())) throw new Exception("Invalid QR image");
    String code;
    try (InputStream input = getContentResolver().openInputStream(uri)) { code = PairingQr.decode(input); }
    String reply = new JSONObject().put("code", code).toString(); runOnUiThread(() -> result.success(reply));
   } catch (Exception failure) { runOnUiThread(() -> result.error("qr_invalid", "Cannot read pairing QR image. Paste the pairing code instead.", null)); }
  });
 }
 private void receiveInvitation(Intent intent) {
  if (Intent.ACTION_VIEW.equals(intent.getAction()) && intent.getData() != null) {
   serviceLink = intent.getDataString(); intent.setData(null); return;
  }
  if ((Intent.ACTION_SEND.equals(intent.getAction()) || Intent.ACTION_SEND_MULTIPLE.equals(intent.getAction()))
      && (intent.hasExtra(Intent.EXTRA_STREAM) || intent.getClipData() != null)) {
   if (sharedFiles.size() < 8) sharedFiles.addLast(new Intent(intent));
   else invitationError = "Finish the current shares before sharing more files.";
   intent.removeExtra(Intent.EXTRA_STREAM); intent.setClipData(null); intent.setAction(null);
   return;
  }
  if (!Intent.ACTION_SEND.equals(intent.getAction())) return;
  invitation = null; invitationError = null;
  if (intent.hasExtra(Intent.EXTRA_STREAM) || intent.getStringExtra(Intent.EXTRA_TEXT) == null) invitationError = "Share or paste a pairing code instead.";
  else {
   String text = intent.getStringExtra(Intent.EXTRA_TEXT);
   if (text.getBytes(StandardCharsets.UTF_8).length > 8192) invitationError = "Pairing code is too large. Share the complete Wispnet code.";
   else invitation = text;
  }
  intent.removeExtra(Intent.EXTRA_TEXT); intent.removeExtra(Intent.EXTRA_STREAM);
 }
 @Override protected void onSaveInstanceState(@NonNull Bundle state) {
  state.putParcelableArrayList("wispnet.shares", new java.util.ArrayList<>(sharedFiles));
  super.onSaveInstanceState(state);
 }
 @Override protected void onNewIntent(Intent intent) { super.onNewIntent(intent); setIntent(intent); receiveInvitation(intent); }
 @Override protected void onDestroy() {
  stopQrCamera();
  if (fileCancellation != null) fileCancellation.cancel();
  if (fileResult != null) { fileResult.error("activity_closed", "File selection interrupted. Select the files again.", null); fileResult = null; }
  if (folderResult != null) { folderResult.error("activity_closed", "Folder selection interrupted. Choose the folder again.", null); folderResult = null; }
  if (qrResult != null) { qrResult.error("activity_closed", "QR request interrupted. Try again.", null); qrResult = null; }
  // Native terminal holders retain their network streams across Flutter activity
  // recreation; platform-view disposal only detaches the viewport and IME.
  network.close(); work.shutdown(); state.shutdown(); super.onDestroy();
 }
}
