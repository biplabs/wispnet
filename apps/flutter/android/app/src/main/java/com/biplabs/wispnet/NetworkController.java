package com.biplabs.wispnet;

import android.content.*;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicBoolean;
import org.json.*;

/** Owns native state and persistent UI records exclusively in the network process. */
final class NetworkController extends ContextWrapper {
 private final AccessService service;
 private final AndroidFiles files;
 private final AndroidDownloads downloads;
 final ExecutorService work = Executors.newFixedThreadPool(4);
 private static final AtomicBoolean pairing = new AtomicBoolean();
 NetworkController(AccessService service) { super(service); this.service = service; this.files = new AndroidFiles(service); this.downloads = new AndroidDownloads(service); }
 void close() { files.close(); downloads.close(); work.shutdown(); }
 void cancelFiles() { files.stop(); downloads.stop(); }
 void filesOnline() { files.online(); downloads.online(); }
 JSONObject dispatch(String method, JSONObject args) throws Exception {
  HostStore store = new HostStore(this);
  switch (method) {
   case "setServiceEnabled": {
    String id = args.getString("service");
    if (!(args.get("enabled") instanceof Boolean)) throw new Exception("Invalid local service");
    JSONObject status = Native.command("local_files_status");
    if (!"files".equals(id)) {
     JSONObject folders = folderServicesStatus(); JSONObject selected = null;
     for (String collection : new String[]{"inboxes","shares"}) {
      JSONArray rows = folders.getJSONArray(collection);
      for (int index=0;index<rows.length();index++) if (id.equals(rows.getJSONObject(index).getString("id"))) selected = rows.getJSONObject(index);
     }
     if (selected == null) throw new Exception("Folder service unavailable");
     if (args.getBoolean("enabled") && !selected.getBoolean("available"))
      throw new Exception("Folder access expired. Choose the folder again.");
     status = folders;
    }
    return Native.request(new JSONObject().put("op", "local_service_enabled")
     .put("id", id).put("incarnation", args.getString("incarnation"))
     .put("enabled", args.getBoolean("enabled")).put("expected_revision", status.getLong("revision")));
   }
   case "receivingStatus": return new AndroidReceiving(this).status();
   case "folderServicesStatus": return folderServicesStatus();
   case "saveInbox": return new AndroidReceiving(this).configureInbox(args);
   case "removeInbox": return new AndroidReceiving(this).removeInbox(args);
   case "sharesStatus": return new AndroidShares(this).status();
   case "prepareShareFolder": return new AndroidShares(this).prepare(android.net.Uri.parse(args.getString("uri")));
   case "saveShare": return new AndroidShares(this).configure(args);
   case "removeShare": return new AndroidShares(this).remove(args);
   case "prepareReceivingFolder": return new AndroidReceiving(this).prepare(android.net.Uri.parse(args.getString("uri")));
   case "prepareDownloadFolder": return downloads.prepare(android.net.Uri.parse(args.getString("uri")));
   case "shareList": return downloads.list(args);
   case "shareGet": return downloads.get(args);
   case "saveReceiving": return new AndroidReceiving(this).configure(args);
   case "fileWorkspaces":
   case "fileSend": {
    if (!service.enabled) throw new Exception("Go online before sending files");
    return "fileSend".equals(method) ? files.send(args) : files.workspaces(args);
   }
   case "fileStatus": return downloads.contains(args.getString("transfer")) ? downloads.status(args.getString("transfer")) : files.status(args.getString("transfer"));
   case "fileCancel": return downloads.contains(args.getString("transfer")) ? downloads.cancel(args.getString("transfer")) : files.cancel(args.getString("transfer"));
   case "fileList": return files.list();
   case "shell_open":
   case "shell_input":
   case "shell_resize":
   case "shell_poll":
   case "shell_close": {
    if (!service.enabled && !"shell_close".equals(method)) throw new Exception("Go online before opening a shell");
    JSONObject request = new JSONObject(args.toString()).put("op", method);
    return Native.request(request);
   }
   case "snapshot": {
    JSONObject order = new JSONObject();
    for (java.util.Map.Entry<String, ?> entry : getSharedPreferences("card-order", 0).getAll().entrySet())
     if (entry.getValue() instanceof String) try { order.put(entry.getKey(), new JSONArray((String)entry.getValue())); } catch (JSONException ignored) {}
    JSONObject hosts = store.all();
    String own = Native.command("identity").getString("peer");
    JSONObject folders = folderServicesStatus();
    JSONArray localServices = new JSONArray();
    JSONArray inboxes = folders.getJSONArray("inboxes");
    for (int index=0;index<inboxes.length();index++) localServices.put(inboxes.getJSONObject(index));
    JSONArray sourceShares = folders.getJSONArray("shares");
    for (int index=0;index<sourceShares.length();index++) localServices.put(sourceShares.getJSONObject(index));
    hosts.put(own, new JSONObject().put("name", DeviceProfile.name(this)).put("local", true)
     .put("state", "available").put("catalog", new JSONObject().put("services", localServices)));
    return new JSONObject().put("hosts", hosts).put("name", DeviceProfile.name(this))
     .put("share_downloads", downloads.snapshots())
     .put("network_pid", android.os.Process.myPid()).put("peer", Native.command("identity").getString("peer")).put("enabled", service.enabled)
     .put("network", service.networkStatus).put("relay", Native.command("relay_status").getString("relay"))
     .put("telemetry", Native.command("telemetry")).put("order", order)
     .put("version", getPackageManager().getPackageInfo(getPackageName(), 0).versionName);
   }
   case "refreshHost": {
    String peer = args.getString("host");
    if (!store.all().has(peer)) throw new Exception("Machine was removed");
    try { store.catalog(peer, Native.request(new JSONObject().put("op", "catalog").put("host", peer)));
     try { store.publicLinks(peer, Native.request(new JSONObject().put("op", "host_public_links").put("host", peer))); } catch (Exception olderHost) {}
     DeviceProfile.syncHost(this, peer);
     try { store.deviceId(peer, Native.request(new JSONObject().put("op", "host_device_id").put("host", peer)).getString("device_id")); } catch (Exception olderHost) {}
    }
    catch (Exception failure) { store.failed(peer, failure); }
    return new JSONObject();
   }
   case "preview": return preview(args.getString("code"));
   case "pair": {
    if (!pairing.compareAndSet(false, true)) throw new Exception("A pairing request is already in progress. Please wait.");
    try {
     String code = normalized(args.getString("code"));
     JSONObject target = preview(code); String peer = target.getString("host");
     String name = target.optString("host_name", "").trim(); if (name.isEmpty()) name = "device-" + peer.substring(0, 8); name = DeviceProfile.readableId(name);
     Native.request(new JSONObject().put("op", "pair").put("name", DeviceProfile.name(this)).put("host", peer).put("invitation", code));
     store.paired(peer, name);
     return new JSONObject().put("host", peer).put("name", name);
    } finally { pairing.set(false); }
   }
   case "removeHost": {
    files.cancelHost(args.getString("host"));
    downloads.cancelHost(args.getString("host"));
    store.remove(args.getString("host"));
    if (store.all().length() == 0) service.stopAccess();
    return new JSONObject();
   }
   case "open": {
    if (!service.enabled) throw new Exception("Start browser access first");
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
    JSONObject reply = new JSONObject().put("port", settings.getInt("registry_port"));
    if (args.isNull("host")) return reply.put("url", "http://wispnet.localhost:" + settings.getInt("registry_port") + "/registry");
    JSONArray routes = data.getJSONArray("routes");
    for (int i = 0; i < routes.length(); i++) { JSONObject row = routes.getJSONObject(i); if (args.getString("host").equals(row.getString("host")) && args.getString("service").equals(row.getString("service"))) reply.put("url", row.getString("url")); }
    return reply;
   }
   case "saveSettings": {
    boolean global = args.isNull("host"); int port = args.getInt("port");
    if (port < 1024 || port > 65535) throw new Exception("Port must be between 1024 and 65535.");
    String name = global ? args.getString("name").trim() : null;
    if (global && !DeviceProfile.valid(name)) throw new Exception("Enter a valid device ID.");
    JSONObject settings = Native.command("gateway_settings").getJSONObject("settings");
    settings.put("registry_port", port).put("service_ports", new JSONArray());
    Native.request(new JSONObject().put("op", "configure_access").put("settings", settings));
    if (global && !name.equals(DeviceProfile.name(this))) { DeviceProfile.save(this, name); work.execute(() -> DeviceProfile.syncAll(this)); }
    return new JSONObject();
   }
   case "resolveLink": {
    if (!service.enabled) throw new Exception("Go online before opening a Wispnet link");
    return Native.request(new JSONObject().put("op", "resolve_link").put("link", args.getString("link")));
   }
   default: throw new Exception("Unsupported UI operation: " + method);
  }
 }
 private JSONObject folderServicesStatus() throws Exception {
  // Both adapters describe one revision before the UI can replace a policy.
  for (int attempt=0;attempt<3;attempt++) {
   JSONObject receiving=new AndroidReceiving(this).inboxesStatus();
   JSONObject sharing=new AndroidShares(this).status();
   if (receiving.getLong("revision")==sharing.getLong("revision"))
    return new JSONObject().put("revision",receiving.getLong("revision"))
        .put("clients",receiving.getJSONArray("clients")).put("inboxes",receiving.getJSONArray("inboxes"))
        .put("shares",sharing.getJSONArray("shares"));
  }
  throw new Exception("Folder permissions changed. Refresh and try again.");
 }
 private String normalized(String raw) throws Exception {
  String code = raw.replaceAll("\\s+", "");
  if (code.getBytes(StandardCharsets.UTF_8).length > 8192 || !code.startsWith("wispnet1:")) throw new Exception("Paste the complete Wispnet pairing code or scan its QR.");
  return code;
 }
 private JSONObject preview(String raw) throws Exception { return Native.request(new JSONObject().put("op", "preview_invitation").put("invitation", normalized(raw))); }
}
