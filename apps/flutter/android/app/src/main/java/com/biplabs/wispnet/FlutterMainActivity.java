package com.biplabs.wispnet;

import android.Manifest;
import android.app.Activity;
import android.content.*;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Bundle;
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
 private String invitation, invitationError, serviceLink;

 @Override public void onCreate(Bundle state) {
  super.onCreate(state);
  network = new NetworkBridge(this);
  receiveInvitation(getIntent());
  if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED)
   requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 101);
 }
 @Override public void configureFlutterEngine(@NonNull FlutterEngine engine) {
  super.configureFlutterEngine(engine);
  new MethodChannel(engine.getDartExecutor().getBinaryMessenger(), "app.wispnet/ui").setMethodCallHandler((call, result) -> {
   final JSONObject arguments;
   try { arguments = new JSONObject(call.arguments == null ? "{}" : (String)call.arguments); }
   catch (Exception invalid) { result.error("invalid_request", "Invalid UI request", null); return; }
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
   if ("scanQr".equals(call.method) || "importQr".equals(call.method)) { startQr(call.method, result); return; }
   if ("openShell".equals(call.method)) {
    try {
     String host = arguments.getString("host"), service = arguments.getString("service"), incarnation = arguments.getString("incarnation");
     if (!host.matches("[0-9a-f]{64}") || !incarnation.matches("[0-9a-f]{64}") || !service.matches("[a-z0-9][a-z0-9._-]{0,63}")) throw new Exception("Invalid shell service");
     startActivity(new Intent(this, com.biplabs.wispnet.shell.ShellActivity.class)
      .putExtra("target", arguments.toString()).putExtra("session", arguments.optString("session", "main")));
     result.success("{}");
    } catch (Exception failure) { result.error("shell_failed", failure.getMessage(), null); }
    return;
   }
   if ("closeShell".equals(call.method)) {
    com.biplabs.wispnet.shell.ShellWorkspace.INSTANCE.closeAll(this);
    result.success("{}");
    return;
   }
   if ("setAccess".equals(call.method)) {
    if (arguments.optBoolean("enabled")) {
     try { startForegroundService(new Intent(this, AccessService.class)); }
     catch (Exception failure) { result.error("access_failed", failure.getMessage(), null); return; }
    } else {
     com.biplabs.wispnet.shell.ShellWorkspace.INSTANCE.closeAll(this);
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
     if ("removeHost".equals(call.method)) {
      com.biplabs.wispnet.shell.ShellWorkspace.INSTANCE.closeHost(this, arguments.optString("host"));
     }
     if ("snapshot".equals(call.method)) {
      reply.put("ui_pid", android.os.Process.myPid());
      JSONObject shell = com.biplabs.wispnet.shell.ShellWorkspace.INSTANCE.snapshotJson();
      if (shell != null) reply.put("shell", shell);
     }
     runOnUiThread(() -> result.success(reply.toString()));
    } catch (Exception failure) {
     String code = failure instanceof NetworkBridge.Failure ? ((NetworkBridge.Failure)failure).code : "operation_failed";
     runOnUiThread(() -> result.error(code, failure.getMessage(), null));
    }
   });
  });
 }
 private void startQr(String method, MethodChannel.Result result) {
  if (qrResult != null) { result.error("qr_busy", "A QR request is already in progress", null); return; }
  try {
   qrResult = result;
   if ("importQr".equals(method)) startActivityForResult(new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("image/*").addCategory(Intent.CATEGORY_OPENABLE), 103);
   else {
    if (!getPackageManager().hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)) throw new Exception("No camera available. Import a QR image or paste a pairing code.");
    com.journeyapps.barcodescanner.ScanOptions options = new com.journeyapps.barcodescanner.ScanOptions().setCaptureActivity(PairingCaptureActivity.class).setDesiredBarcodeFormats(com.journeyapps.barcodescanner.ScanOptions.QR_CODE).setBeepEnabled(false).setBarcodeImageEnabled(false).setTimeout(60000).setPrompt("Scan your computer’s Wispnet QR to pair.");
    startActivityForResult(options.createScanIntent(this), 102);
   }
  } catch (Exception failure) { qrResult = null; result.error("qr_unavailable", failure.getMessage(), null); }
 }
 @Override protected void onActivityResult(int request, int resultCode, Intent data) {
  super.onActivityResult(request, resultCode, data);
  if ((request != 102 && request != 103) || qrResult == null) return;
  MethodChannel.Result result = qrResult; qrResult = null;
  if (resultCode != Activity.RESULT_OK || data == null) { result.success("{}"); return; }
  work.execute(() -> {
   try {
    String code;
    if (request == 102) code = data.getStringExtra("SCAN_RESULT");
    else { Uri uri = data.getData(); if (uri == null || !"content".equals(uri.getScheme())) throw new Exception("Invalid QR image"); try (InputStream input = getContentResolver().openInputStream(uri)) { code = PairingQr.decode(input); } }
    String reply = new JSONObject().put("code", code).toString(); runOnUiThread(() -> result.success(reply));
   } catch (Exception failure) { runOnUiThread(() -> result.error("qr_invalid", "Cannot read pairing QR image. Paste the pairing code instead.", null)); }
  });
 }
 private void receiveInvitation(Intent intent) {
  if (Intent.ACTION_VIEW.equals(intent.getAction()) && intent.getData() != null) {
   serviceLink = intent.getDataString(); intent.setData(null); return;
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
 @Override protected void onNewIntent(Intent intent) { super.onNewIntent(intent); setIntent(intent); receiveInvitation(intent); }
 @Override protected void onDestroy() {
  if (qrResult != null) { qrResult.error("activity_closed", "QR request interrupted. Try again.", null); qrResult = null; }
  com.biplabs.wispnet.shell.ShellWorkspace.INSTANCE.closeAll(this);
  network.close(); work.shutdown(); state.shutdown(); super.onDestroy();
 }
}
