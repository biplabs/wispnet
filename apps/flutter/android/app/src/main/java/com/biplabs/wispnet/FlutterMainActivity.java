package com.biplabs.wispnet;

import android.Manifest;
import android.app.Activity;
import android.content.*;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Bundle;
import android.view.WindowManager;
import androidx.annotation.NonNull;
import io.flutter.embedding.android.FlutterActivity;
import io.flutter.embedding.engine.FlutterEngine;
import io.flutter.plugin.common.MethodChannel;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicBoolean;
import org.json.*;

/** Android lifecycle/storage adapter. Flutter never handles the protected key. */
public final class FlutterMainActivity extends FlutterActivity {
 private final ExecutorService work = Executors.newFixedThreadPool(4);
 private final ExecutorService state = Executors.newSingleThreadExecutor();
 private static final AtomicBoolean pairing = new AtomicBoolean();
 private MethodChannel.Result qrResult;
 private String invitation, invitationError;

 @Override public void onCreate(Bundle state) {
  super.onCreate(state);
  getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);
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
   if ("takeInvitation".equals(call.method)) {
    JSONObject reply = new JSONObject();
    try { if (invitation != null) reply.put("code", invitation); if (invitationError != null) reply.put("error", invitationError); invitation = null; invitationError = null; result.success(reply.toString()); }
    catch (JSONException failure) { result.error("invitation_invalid", failure.getMessage(), null); }
    return;
   }
   if ("scanQr".equals(call.method) || "importQr".equals(call.method)) { startQr(call.method, result); return; }
   if ("setAccess".equals(call.method)) {
    try {
     if (arguments.getBoolean("enabled")) startForegroundService(new Intent(this, AccessService.class));
     else stopService(new Intent(this, AccessService.class));
     result.success("{}");
    } catch (Exception failure) { result.error("access_failed", failure.getMessage(), null); }
    return;
   }
   if ("launch".equals(call.method)) {
    try {
     if (!AccessService.enabled) throw new Exception("Start browser access first");
     Uri url = Uri.parse(arguments.getString("url"));
     if (!"http".equals(url.getScheme()) && !"https".equals(url.getScheme())) throw new Exception("Invalid service URL");
     startActivity(new Intent(Intent.ACTION_VIEW, url).addCategory(Intent.CATEGORY_BROWSABLE)); result.success("{}");
    } catch (Exception failure) { result.error("launch_failed", failure.getMessage(), null); }
    return;
   }
   ExecutorService executor = java.util.Set.of("snapshot", "settings", "saveOrder", "preview").contains(call.method) ? state : work;
   executor.execute(() -> {
    try {
     Native.ensure(this);
     JSONObject reply = dispatch(call.method, arguments);
     runOnUiThread(() -> result.success(reply.toString()));
    } catch (Exception failure) {
     String code = failure instanceof Native.Failure ? ((Native.Failure)failure).code : "operation_failed";
     runOnUiThread(() -> result.error(code, failure.getMessage(), null));
    }
   });
  });
 }
 private JSONObject dispatch(String method, JSONObject args) throws Exception {
  HostStore store = new HostStore(this);
  switch (method) {
   case "snapshot": {
    JSONObject order = new JSONObject();
    for (java.util.Map.Entry<String, ?> entry : getSharedPreferences("card-order", 0).getAll().entrySet())
     if (entry.getValue() instanceof String) try { order.put(entry.getKey(), new JSONArray((String)entry.getValue())); } catch (JSONException ignored) {}
    return new JSONObject().put("hosts", store.all()).put("name", DeviceProfile.name(this))
     .put("peer", Native.command("identity").getString("peer")).put("enabled", AccessService.enabled)
     .put("network", AccessService.networkStatus).put("relay", Native.command("relay_status").getString("relay"))
     .put("telemetry", Native.command("telemetry")).put("order", order)
     .put("version", getPackageManager().getPackageInfo(getPackageName(), 0).versionName);
   }
   case "refreshHost": {
    String peer = args.getString("host");
    if (!store.all().has(peer)) throw new Exception("Machine was removed");
    try { store.catalog(peer, Native.request(new JSONObject().put("op", "catalog").put("host", peer))); DeviceProfile.syncHost(this, peer); }
    catch (Exception failure) { store.failed(peer, failure); }
    return new JSONObject();
   }
   case "preview": return preview(args.getString("code"));
   case "pair": {
    if (!pairing.compareAndSet(false, true)) throw new Exception("A pairing request is already in progress. Please wait.");
    try {
     String code = normalized(args.getString("code"));
     JSONObject target = preview(code); String peer = target.getString("host");
     String name = target.optString("host_name", "").trim(); if (name.isEmpty()) name = "Host " + peer.substring(0, 8);
     Native.request(new JSONObject().put("op", "pair").put("name", DeviceProfile.name(this)).put("host", peer).put("invitation", code));
     store.paired(peer, name);
     return new JSONObject().put("host", peer).put("name", name);
    } finally { pairing.set(false); }
   }
   case "removeHost": {
    store.remove(args.getString("host"));
    if (store.all().length() == 0) stopService(new Intent(this, AccessService.class));
    return new JSONObject();
   }
   case "open": {
    if (!AccessService.enabled) throw new Exception("Start browser access first");
    String peer = args.getString("host");
    try {
     return Native.request(new JSONObject().put("op", "open").put("host", peer)
      .put("host_name", store.all().getJSONObject(peer).getString("name"))
      .put("service", args.getString("service")).put("incarnation", args.getString("incarnation")));
    } catch (Exception failure) { store.failed(peer, failure); throw failure; }
   }
   case "saveOrder": {
    String group = args.getString("group"); JSONArray ids = args.getJSONArray("ids");
    if (!(group.equals("machines") || group.matches("services/[0-9a-f]{64}")) || ids.length() > 128) throw new Exception("Invalid card order");
    if (!getSharedPreferences("card-order", 0).edit().putString(group, ids.toString()).commit()) throw new Exception("Cannot save card order");
    return new JSONObject();
   }
   case "connectionDiagnostics": {
    android.net.ConnectivityManager cm = getSystemService(android.net.ConnectivityManager.class);
    android.net.Network active = cm.getActiveNetwork();
    android.net.NetworkCapabilities caps = cm.getNetworkCapabilities(active);
    String network = caps == null ? "Offline" : caps.hasTransport(android.net.NetworkCapabilities.TRANSPORT_WIFI) ? "Wi-Fi" : caps.hasTransport(android.net.NetworkCapabilities.TRANSPORT_CELLULAR) ? "Cellular" : "Other network";
    JSONArray peers = new JSONArray(); JSONObject records = store.all();
    for (java.util.Iterator<String> keys = records.keys(); keys.hasNext();) {
     String host = keys.next(); if (!host.equals(args.optString("host"))) continue; JSONObject row = new JSONObject().put("name", records.getJSONObject(host).optString("name", host)).put("host", host);
     try { row.put("report", Native.request(new JSONObject().put("op", "diagnose_connection").put("host", host))); }
     catch (Exception failure) { row.put("error", failure.getMessage()); }
     peers.put(row);
    }
    android.net.Network after = cm.getActiveNetwork();
    return new JSONObject().put("network", network).put("vpn", caps != null && caps.hasTransport(android.net.NetworkCapabilities.TRANSPORT_VPN))
     .put("network_changed", !java.util.Objects.equals(active, after)).put("peers", peers);
   }
   case "settings": {
    JSONObject data = Native.command("gateway_settings"), settings = data.getJSONObject("settings");
    if (args.isNull("host")) return new JSONObject().put("port", settings.getInt("registry_port")).put("url", "http://registry.wispnet.localhost:" + settings.getInt("registry_port"));
    String peer = args.getString("host"), service = args.getString("service"); JSONObject reply = new JSONObject();
    JSONArray ports = settings.getJSONArray("service_ports");
    for (int i = 0; i < ports.length(); i++) { JSONObject row = ports.getJSONObject(i); if (peer.equals(row.getString("host")) && service.equals(row.getString("service"))) reply.put("port", row.getInt("port")); }
    JSONArray routes = data.getJSONArray("routes");
    for (int i = 0; i < routes.length(); i++) { JSONObject row = routes.getJSONObject(i); if (peer.equals(row.getString("host")) && service.equals(row.getString("service"))) reply.put("url", row.getString("url")); }
    return reply;
   }
   case "saveSettings": {
    boolean global = args.isNull("host"); int port = args.getInt("port");
    if ((global || port != 0) && (port < 1024 || port > 65535)) throw new Exception("Port must be between 1024 and 65535.");
    String name = global ? args.getString("name").trim() : null;
    if (global && !DeviceProfile.valid(name)) throw new Exception("Enter a name up to 128 UTF-8 bytes, without control characters.");
    JSONObject settings = Native.command("gateway_settings").getJSONObject("settings");
    boolean changed = !global || settings.getInt("registry_port") != port;
    if (global) settings.put("registry_port", port);
    else {
     String peer = args.getString("host"), service = args.getString("service"); JSONArray old = settings.getJSONArray("service_ports"), ports = new JSONArray();
     for (int i = 0; i < old.length(); i++) { JSONObject row = old.getJSONObject(i); if (!peer.equals(row.getString("host")) || !service.equals(row.getString("service"))) ports.put(row); }
     if (port != 0) ports.put(new JSONObject().put("host", peer).put("service", service).put("port", port));
     settings.put("service_ports", ports);
    }
    if (changed) Native.request(new JSONObject().put("op", "configure_access").put("settings", settings));
    if (global && !name.equals(DeviceProfile.name(this))) { DeviceProfile.save(this, name); if (!work.isShutdown()) work.execute(() -> DeviceProfile.syncAll(this)); }
    return new JSONObject();
   }
   default: throw new Exception("Unsupported UI operation: " + method);
  }
 }
 private String normalized(String raw) throws Exception {
  String code = raw.replaceAll("\\s+", "");
  if (code.getBytes(StandardCharsets.UTF_8).length > 8192 || !code.startsWith("wispnet1:")) throw new Exception("Paste the complete Wispnet pairing code or scan its QR.");
  return code;
 }
 private JSONObject preview(String raw) throws Exception { return Native.request(new JSONObject().put("op", "preview_invitation").put("invitation", normalized(raw))); }
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
  work.shutdown(); state.shutdown(); super.onDestroy();
 }
}
