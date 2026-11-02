package com.biplabs.wispnet;

import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.Uri;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.Executor;
import java.util.concurrent.Executors;
import org.json.JSONArray;
import org.json.JSONObject;

/** Private owner controls. Provider roots stay in JNI and persisted URI grants;
 * cross-host operations use only an existing, freshly checked Shell capability. */
final class AndroidOwner {
 interface NativeRequest { JSONObject call(JSONObject request) throws Exception; }
 interface FolderStatus { JSONObject read() throws Exception; }
 interface TargetSource { JSONArray read() throws Exception; }

 private static final Object JOURNAL_LOCK = new Object();
 private static final Set<String> RUNNING = new HashSet<>();
 private static final int MAX_RECORDS = 100, MAX_RECORD_BYTES = 65 * 1024, MAX_JOURNAL_BYTES = 192 * 1024;
 private static final Executor APPLY = Executors.newFixedThreadPool(2, runnable -> {
  Thread thread = new Thread(runnable, "wispnet-owner-operation"); thread.setDaemon(true); return thread;
 });
 private final Context context;
 private final SharedPreferences journal;
 private final NativeRequest nativeRequest;
 private final Executor executor;
 private final TargetSource targetSource;

 AndroidOwner(Context context) {
  this(context, context.getSharedPreferences("owner-operations", Context.MODE_PRIVATE), Native::request, APPLY, null);
 }
 AndroidOwner(Context context, SharedPreferences journal, NativeRequest nativeRequest, Executor executor, TargetSource targets) {
  this.context = context.getApplicationContext(); this.journal = journal;
  this.nativeRequest = nativeRequest; this.executor = executor; this.targetSource = targets;
 }

 JSONObject status(FolderStatus provider) throws Exception {
  for (int attempt = 0; attempt < 3; attempt++) {
   JSONObject raw = call("local_owner_status");
   JSONObject folders = provider.read();
   if (raw.getJSONObject("host").getLong("revision") == folders.getLong("revision"))
    return combineStatus(raw, folders);
  }
  throw new Exception("Service permissions changed. Refresh and try again.");
 }

 static JSONObject combineStatus(JSONObject raw, JSONObject folders) throws Exception {
  JSONObject result = new JSONObject(raw.toString()), host = result.getJSONObject("host");
  if (host.getLong("revision") != folders.getLong("revision"))
   throw new Exception("Service permissions changed. Refresh and try again.");
  Map<String, JSONObject> descriptors = new LinkedHashMap<>();
  for (String collection : new String[]{"inboxes", "shares"}) {
   JSONArray rows = folders.getJSONArray(collection);
   for (int index = 0; index < rows.length(); index++) {
    JSONObject row = rows.getJSONObject(index); descriptors.put(row.getString("id"), row);
   }
  }
  JSONArray visible = new JSONArray(), services = host.getJSONArray("services");
  for (int index = 0; index < services.length(); index++) {
   JSONObject service = services.getJSONObject(index);
   String id = service.getString("id"), kind = service.getString("kind");
   if ("shell".equals(kind)) continue;
   boolean folder = Set.of("files", "file_inbox", "file_share").contains(kind);
   JSONObject descriptor = descriptors.get(id);
   if ("files".equals(kind) && descriptor == null) continue;
   if (folder) {
    if (descriptor == null || !service.getString("incarnation").equals(descriptor.getString("incarnation")))
     throw new Exception("Folder permissions changed. Refresh and try again.");
    for (String field : new String[]{"folder", "label", "available"}) service.put(field, descriptor.get(field));
    service.remove("backend");
   }
   visible.put(service);
  }
  host.put("services", visible);
  // Keep every compatibility export on grant-only edits, including old multi-root configurations.
  if (folders.has("file_exports")) host.put("file_exports", new JSONObject(folders.getJSONObject("file_exports").toString()));
  return result;
 }

 JSONObject request(JSONObject payload) throws Exception {
  String op = text(payload, "op");
  switch (op) {
   case "mutate":
    keys(payload, Set.of("op", "operation"));
    return nativeRequest.call(new JSONObject().put("op", "local_owner_mutate").put("operation", payload.getJSONObject("operation")));
   case "invite_qr":
    keys(payload, Set.of("op"));
    return call("local_owner_invite");
   case "configure_public": {
    keys(payload, Set.of("op", "service", "incarnation", "enabled", "name"));
    String service = text(payload, "service"), incarnation = text(payload, "incarnation");
    if (!service.matches("[a-z0-9][a-z0-9._-]{0,63}") || !incarnation.matches("[0-9a-f]{64}")
        || !(payload.get("enabled") instanceof Boolean)) throw new Exception("Invalid public service setting");
    return nativeRequest.call(new JSONObject().put("op", "configure_public").put("service", service)
      .put("incarnation", incarnation).put("enabled", payload.getBoolean("enabled"))
      .put("name", DeviceProfile.name(context)));
   }
   default: throw new Exception("Unsupported owner operation");
  }
 }

 JSONObject launchPublic(JSONObject args) throws Exception {
  String id = text(args, "service");
  JSONArray services = call("public_status").getJSONArray("services");
  for (int index = 0; index < services.length(); index++) {
   JSONObject publication = services.getJSONObject(index);
   if (!id.equals(publication.optString("service")) || !Boolean.TRUE.equals(publication.opt("enabled"))) continue;
   String url = publicUrl(text(publication, "url"));
   context.startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(url))
     .addCategory(Intent.CATEGORY_BROWSABLE).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
   return new JSONObject().put("url", url);
  }
  throw new Exception("Public URL unavailable");
 }

 static String publicUrl(String value) throws Exception {
  if (value.length() > 8192 || value.codePoints().anyMatch(Character::isISOControl)) throw new Exception("Invalid public URL");
  URI uri = new URI(value);
  if (!("https".equals(uri.getScheme()) || "http".equals(uri.getScheme())) || uri.getHost() == null
      || uri.getHost().isEmpty() || uri.getUserInfo() != null || uri.getPort() == 0 || uri.getPort() > 65535)
   throw new Exception("Invalid public URL");
  return uri.toASCIIString();
 }

 Object data(JSONObject args) throws Exception {
  switch (text(args, "op")) {
   case "operations": return operations();
   case "targets": return targets();
   case "start": return start(args.getJSONObject("plan"));
   case "retry": {
    String id = identifier(args, "id");
    JSONObject record;
    synchronized (JOURNAL_LOCK) { record = record(records(), id); }
    schedule(id);
    return record;
   }
   default: throw new Exception("Unsupported owner journal operation");
  }
 }

 private JSONArray operations() throws Exception {
  synchronized (JOURNAL_LOCK) {
   JSONObject all = records(); ArrayList<JSONObject> values = new ArrayList<>();
   for (Iterator<String> ids = all.keys(); ids.hasNext();) values.add(record(all, ids.next()));
   values.sort((left, right) -> Long.compare(right.optLong("created_at"), left.optLong("created_at")));
   JSONArray result = new JSONArray(); for (JSONObject value : values) result.put(value); return result;
  }
 }

 private JSONArray targets() throws Exception {
  if (targetSource != null) return targetSource.read();
  String own = identifier(call("identity"), "peer");
  JSONArray result = new JSONArray().put(new JSONObject().put("host", own).put("name", DeviceProfile.name(context))
      .put("owner_access", "This device"));
  JSONObject hosts = new HostStore(context).all();
  for (Iterator<String> ids = hosts.keys(); ids.hasNext();) {
   String host = ids.next(); if (own.equals(host) || !host.matches("[0-9a-f]{64}")) continue;
   JSONObject row = hosts.getJSONObject(host), catalog = row.optJSONObject("catalog");
   if (catalog == null || !host.equals(catalog.optString("host"))) continue;
   JSONArray services = catalog.optJSONArray("services"); if (services == null) continue;
   for (int index = 0; index < services.length(); index++) {
    JSONObject service = services.getJSONObject(index);
    if ("shell".equals(service.optString("kind")) && !Boolean.FALSE.equals(service.opt("enabled"))
        && service.optString("incarnation").matches("[0-9a-f]{64}")) {
     result.put(new JSONObject().put("host", host).put("name", row.optString("name", "device-" + host.substring(0, 8)))
         .put("owner_access", "Granted Shell")); break;
    }
   }
  }
  return result;
 }

 private JSONObject start(JSONObject plan) throws Exception {
  keys(plan, Set.of("action", "targets"));
  JSONObject action = action(plan.getJSONObject("action"));
  JSONArray selected = plan.getJSONArray("targets"), known = targets();
  if (selected.length() == 0 || selected.length() > 32) throw new Exception("Choose up to 32 owner targets");
  Map<String, JSONObject> eligible = new LinkedHashMap<>();
  for (int index = 0; index < known.length(); index++) {
   JSONObject target = known.getJSONObject(index); eligible.put(identifier(target, "host"), target);
  }
  JSONArray pending = new JSONArray(); Set<String> included = new HashSet<>();
  for (int index = 0; index < selected.length(); index++) {
   JSONObject target = selected.getJSONObject(index); keys(target, Set.of("host", "name", "owner_access"));
   String host = identifier(target, "host");
   if (!eligible.containsKey(host) || !included.add(host)) throw new Exception("Review the currently available owner targets");
   JSONObject row = new JSONObject(eligible.get(host).toString());
   row.put("state", "pending").put("reason", "Not yet applied").put("revision", JSONObject.NULL); pending.put(row);
  }
  byte[] random = new byte[32]; new SecureRandom().nextBytes(random); String id = java.util.HexFormat.of().formatHex(random);
  JSONObject record = new JSONObject().put("id", id).put("action", action).put("targets", pending)
      .put("created_at", System.currentTimeMillis() / 1000);
  synchronized (JOURNAL_LOCK) {
   JSONObject all = records();
   if (all.length() >= MAX_RECORDS) throw new Exception("Owner operation history is full");
   all.put(id, record); save(all);
  }
  // Return immediately after durable intent. Up to 32 remote acknowledgements
  // can outlast a Binder call, and retries must reuse this same operation ID.
  schedule(id); return new JSONObject(record.toString());
 }

 private void schedule(String id) throws Exception {
  synchronized (JOURNAL_LOCK) {
   JSONObject record = record(records(), id);
   boolean pending = false; JSONArray targets = record.getJSONArray("targets");
   for (int index = 0; index < targets.length(); index++) if ("pending".equals(targets.getJSONObject(index).optString("state"))) pending = true;
   if (!pending || !RUNNING.add(id)) return;
  }
  try { executor.execute(() -> { try { apply(id); } finally { synchronized (JOURNAL_LOCK) { RUNNING.remove(id); } } }); }
  catch (RuntimeException unavailable) { synchronized (JOURNAL_LOCK) { RUNNING.remove(id); } throw new Exception("Owner operation is saved. Retry it from history."); }
 }

 private void apply(String id) {
  try {
   JSONObject saved;
   synchronized (JOURNAL_LOCK) { saved = record(records(), id); }
   JSONArray targets = saved.getJSONArray("targets");
   for (int index = 0; index < targets.length(); index++) {
    JSONObject target = targets.getJSONObject(index);
    if (!"pending".equals(target.getString("state"))) continue;
    String host = target.getString("host"), state = "pending", reason = "Acknowledgement unconfirmed";
    Object revision = JSONObject.NULL;
    try {
     JSONObject receipt = nativeRequest.call(new JSONObject().put("op", "local_owner_apply").put("id", id)
         .put("host", host).put("action", saved.getJSONObject("action")));
     if (!id.equals(receipt.optString("id")) || !host.equals(receipt.optString("host"))
         || !Set.of("applied", "rejected", "pending").contains(receipt.optString("state")))
      throw new Exception("Mismatched owner acknowledgement");
     state = receipt.getString("state");
     if (!receipt.isNull("reason")) {
      Object value = receipt.get("reason");
      if (!(value instanceof String)) throw new Exception("Invalid owner acknowledgement reason");
      reason = boundedReason((String)value);
     } else reason = "applied".equals(state) ? "Applied" : "Acknowledgement unconfirmed";
     if (!receipt.isNull("revision")) {
      Object value = receipt.get("revision");
      if (!(value instanceof Integer || value instanceof Long) || receipt.getLong("revision") < 0)
       throw new Exception("Invalid owner acknowledgement revision");
      revision = value;
     }
     if ("applied".equals(state) && revision == JSONObject.NULL)
      throw new Exception("Owner acknowledgement revision unavailable");
    } catch (Exception unconfirmed) { state = "pending"; revision = JSONObject.NULL; reason = boundedReason(unconfirmed.getMessage()); }
    synchronized (JOURNAL_LOCK) {
     JSONObject all = records(), current = record(all, id);
     JSONObject row = current.getJSONArray("targets").getJSONObject(index);
     if (!host.equals(row.getString("host"))) throw new Exception("Owner journal target changed");
     row.put("state", state).put("reason", reason).put("revision", revision);
     all.put(id, current); save(all);
    }
   }
  } catch (Exception unconfirmed) {
   // The durable row remains pending when storage or acknowledgement fails.
   // Reapplying its original ID is safe even if the native owner already acted.
  }
 }

 private JSONObject records() throws Exception {
  String raw = journal.getString("records", "{}");
  if (raw == null || bytes(raw) > MAX_JOURNAL_BYTES) throw new Exception("Owner operation journal unavailable");
  JSONObject result = new JSONObject(raw);
  if (result.length() > MAX_RECORDS) throw new Exception("Owner operation journal unavailable");
  return result;
 }
 private static JSONObject record(JSONObject all, String id) throws Exception {
  if (!id.matches("[0-9a-f]{64}")) throw new Exception("Invalid owner operation ID");
  JSONObject record = all.getJSONObject(id);
  if (!id.equals(record.getString("id")) || bytes(record.toString()) > MAX_RECORD_BYTES) throw new Exception("Owner operation journal unavailable");
  return new JSONObject(record.toString());
 }
 private void save(JSONObject records) throws Exception {
  if (records.length() > MAX_RECORDS || bytes(records.toString()) > MAX_JOURNAL_BYTES) throw new Exception("Owner operation journal is full");
  for (Iterator<String> ids = records.keys(); ids.hasNext();) record(records, ids.next());
  if (!journal.edit().putString("records", records.toString()).commit()) throw new Exception("Cannot save owner operation journal");
 }
 private JSONObject call(String op) throws Exception { return nativeRequest.call(new JSONObject().put("op", op)); }
 private static int bytes(String value) { return value.getBytes(StandardCharsets.UTF_8).length; }
 private static String text(JSONObject value, String key) throws Exception {
  Object item = value.get(key); if (!(item instanceof String)) throw new Exception("Invalid owner request"); return (String)item;
 }
 private static String identifier(JSONObject value, String key) throws Exception {
  String id = text(value, key); if (!id.matches("[0-9a-f]{64}")) throw new Exception("Invalid owner identifier"); return id;
 }
 private static void keys(JSONObject value, Set<String> allowed) throws Exception {
  for (Iterator<String> keys = value.keys(); keys.hasNext();) if (!allowed.contains(keys.next())) throw new Exception("Invalid owner request field");
 }
 private static JSONObject action(JSONObject value) throws Exception {
  String op = text(value, "op"), peer = identifier(value, "peer");
  if (!Set.of("approve", "revoke").contains(op)) throw new Exception("Unsupported owner action");
  keys(value, "approve".equals(op) ? Set.of("op", "peer", "name") : Set.of("op", "peer"));
  JSONObject result = new JSONObject().put("op", op).put("peer", peer);
  if ("approve".equals(op)) {
   String name = text(value, "name"); if (!DeviceProfile.valid(name)) throw new Exception("Invalid device ID"); result.put("name", name);
  }
  return result;
 }
 private static String boundedReason(String message) {
  if (message == null || message.isEmpty()) return "Acknowledgement unconfirmed";
  String clean = message.replaceAll("[\\p{Cntrl}]", " "); return clean.substring(0, Math.min(512, clean.length()));
 }
}
