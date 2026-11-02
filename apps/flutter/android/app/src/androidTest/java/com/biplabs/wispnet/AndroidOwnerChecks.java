package com.biplabs.wispnet;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.content.SharedPreferences;
import android.os.Bundle;
import java.util.ArrayList;
import org.json.JSONArray;
import org.json.JSONObject;

/** Fake-native checks only: no network, grants, shell command or owner mutation is executed. */
final class AndroidOwnerChecks {
 private final Instrumentation test;
 private final JSONArray checks = new JSONArray();
 private static final String OWN = "a".repeat(64), REMOTE = "b".repeat(64), PEER = "c".repeat(64);
 AndroidOwnerChecks(Instrumentation test) { this.test = test; }

 void run() {
  Bundle result = new Bundle(); SharedPreferences journal = test.getTargetContext().getSharedPreferences("owner-test-" + System.nanoTime(), Context.MODE_PRIVATE);
  try {
   statusMerge(); confirmedPins(); publicUrls(); journal(journal); missingAcknowledgement(journal); requestRouting(journal);
   result.putString("owner", new JSONObject().put("checks", checks).toString()); test.finish(Activity.RESULT_OK, result);
  } catch (Exception failure) {
   result.putString("error", failure.toString()); test.finish(Activity.RESULT_CANCELED, result);
  } finally { journal.edit().clear().commit(); }
 }

 private void statusMerge() throws Exception {
  JSONArray nativeServices = new JSONArray().put(service("shell", "shell")).put(service("files", "files"))
    .put(service("website", "http")).put(service("incoming", "file_inbox").put("enabled", false).put("allow", new JSONArray().put(PEER)));
  JSONObject raw = new JSONObject().put("host", new JSONObject().put("host", OWN).put("revision", 4)
    .put("services", nativeServices))
    .put("invitations", new JSONArray()).put("public_access", new JSONObject().put("services", new JSONArray()));
  JSONObject descriptor = service("incoming", "file_inbox").put("folder", "opaque-root").put("label", "Selected folder")
    .put("available", false).put("enabled", true).put("allow", new JSONArray());
  JSONObject folders = new JSONObject().put("revision", 4).put("inboxes", new JSONArray().put(descriptor)).put("shares", new JSONArray())
    .put("file_exports", new JSONObject().put("incoming", "opaque-one").put("archive", "opaque-two"));
  JSONObject merged = AndroidOwner.combineStatus(raw, folders);
  JSONArray services = merged.getJSONObject("host").getJSONArray("services");
  require(services.length() == 2 && "website".equals(services.getJSONObject(0).getString("id")), "Shell and unconfigured Files must be hidden");
  JSONObject inbox = services.getJSONObject(1);
  require("Selected folder".equals(inbox.getString("label")) && !inbox.getBoolean("available"), "SAF metadata missing");
  require(!inbox.getBoolean("enabled") && PEER.equals(inbox.getJSONArray("allow").getString(0)), "Provider metadata replaced native permissions");
  require(merged.getJSONObject("host").getJSONObject("file_exports").length() == 2, "Legacy exports were lost");
  require(raw.getJSONObject("host").getJSONArray("services").length() == 4, "Merge mutated native source");
  folders.put("revision", 5);
  rejected(() -> AndroidOwner.combineStatus(raw, folders));
  folders.put("revision", 4); descriptor.put("incarnation", "e".repeat(64));
  rejected(() -> AndroidOwner.combineStatus(raw, folders));
  checks.put("owner/SAF snapshots agree on revision and incarnation without replacing grants");
 }

 private void publicUrls() throws Exception {
  require("https://public.example/service".equals(AndroidOwner.publicUrl("https://public.example/service")), "HTTPS public URL rejected");
  require("http://public.example:8080/".equals(AndroidOwner.publicUrl("http://public.example:8080/")), "HTTP public URL rejected");
  for (String invalid : new String[]{"javascript:alert(1)", "https://user:pass@public.example/", "https://public.example:70000/", "https://public.example:0/", "https://public.example/\nnext", "https:///missing-host"})
   rejected(() -> AndroidOwner.publicUrl(invalid));
  checks.put("public handoff accepts only parsed HTTP(S) URLs without credentials or controls");
 }

 private void confirmedPins() throws Exception {
  String old = "d".repeat(64), unnamed = "e".repeat(64);
  JSONObject cached = new JSONObject().put("name", "existing-host").put("state", "available")
    .put("catalog", new JSONObject().put("host", old).put("services", new JSONArray().put(service("web", "http"))));
  JSONObject original = new JSONObject().put(old, cached);
  JSONArray pins = new JSONArray().put(OWN).put(REMOTE).put(REMOTE).put("malformed").put(42).put(unnamed);
  JSONArray clients = new JSONArray().put(new JSONObject().put("peer", REMOTE).put("name", "desktop-home"))
    .put(new JSONObject().put("peer", PEER).put("name", "approved-client-only"))
    .put(new JSONObject().put("peer", unnamed).put("name", "Invalid Device Name"));
  JSONObject merged = HostStore.mergeConfirmed(original, pins, clients, OWN);
  require(merged.length() == 3 && merged.has(REMOTE) && !merged.has(OWN) && !merged.has(PEER), "Directory import must use confirmed pins only");
  require(cached.toString().equals(merged.getJSONObject(old).toString()) && original.length() == 1, "Confirmed import replaced existing catalog or state");
  JSONObject added = merged.getJSONObject(REMOTE);
  require("desktop-home".equals(added.getString("name")) && "unverified".equals(added.getString("state")) && !added.has("catalog"), "Incoming host must start unverified without inferred services");
  require(("device-" + unnamed.substring(0,8)).equals(merged.getJSONObject(unnamed).getString("name")), "Invalid client name must use bounded fallback");
  JSONObject full = new JSONObject();
  for(int index=1;index<=128;index++) full.put(String.format(java.util.Locale.ROOT,"%064x",index),new JSONObject().put("name","host-"+index).put("state","available"));
  require(full.toString().equals(HostStore.mergeConfirmed(full,pins,clients,OWN).toString()), "A full cache must retain its current directory");
  checks.put("incoming confirmed pins join the directory without inferring approval, catalogs or grants; existing and full caches stay intact");
 }

 private void journal(SharedPreferences preferences) throws Exception {
  ArrayList<Runnable> queued = new ArrayList<>(); ArrayList<JSONObject> calls = new ArrayList<>(); boolean[] online = {false};
  JSONArray targets = new JSONArray().put(target(OWN, "This device")).put(target(REMOTE, "Granted Shell"));
  AndroidOwner.NativeRequest fake = request -> {
   require("local_owner_apply".equals(request.getString("op")), "Unexpected native operation");
   require(new JSONObject(preferences.getString("records", "{}")).has(request.getString("id")), "Native apply preceded durable intent");
   calls.add(new JSONObject(request.toString()));
   if (REMOTE.equals(request.getString("host")) && !online[0]) throw new Exception("Fixture offline");
   return new JSONObject().put("host", request.getString("host")).put("id", request.getString("id")).put("state", "applied").put("revision", 9);
  };
  AndroidOwner owner = new AndroidOwner(test.getTargetContext(), preferences, fake, queued::add, () -> targets);
  JSONObject action = new JSONObject().put("op", "approve").put("peer", PEER).put("name", "test-client");
  JSONObject started = (JSONObject)owner.data(new JSONObject().put("op", "start").put("plan", new JSONObject().put("action", action).put("targets", targets)));
  String id = started.getString("id");
  require(id.matches("[0-9a-f]{64}") && calls.isEmpty() && queued.size() == 1, "Start must return durable pending intent before dispatch");
  queued.remove(0).run();
  JSONArray rows = ((JSONArray)owner.data(new JSONObject().put("op", "operations"))).getJSONObject(0).getJSONArray("targets");
  require("applied".equals(rows.getJSONObject(0).getString("state")) && "pending".equals(rows.getJSONObject(1).getString("state")), "Partial acknowledgement lost");
  online[0] = true;
  AndroidOwner restarted = new AndroidOwner(test.getTargetContext(), preferences, fake, queued::add, () -> targets);
  restarted.data(new JSONObject().put("op", "retry").put("id", id)); queued.remove(0).run();
  require(calls.size() == 3 && REMOTE.equals(calls.get(2).getString("host")) && id.equals(calls.get(2).getString("id")), "Retry must reuse the ID and skip acknowledged targets");
  restarted.data(new JSONObject().put("op", "retry").put("id", id)); require(queued.isEmpty(), "Completed operations must not reapply");
  int saved = new JSONObject(preferences.getString("records", "{}")).length();
  JSONArray unknown = new JSONArray().put(target("d".repeat(64), "Granted Shell"));
  rejected(() -> restarted.data(new JSONObject().put("op", "start").put("plan", new JSONObject().put("action", action).put("targets", unknown))));
  rejected(() -> restarted.data(new JSONObject().put("op", "start").put("plan", new JSONObject().put("action", action).put("targets", new JSONArray().put(targets.getJSONObject(0)).put(targets.getJSONObject(0))))));
  rejected(() -> restarted.data(new JSONObject().put("op", "start").put("plan", new JSONObject().put("action", new JSONObject().put("op", "shell").put("peer", PEER)).put("targets", targets))));
  require(saved == new JSONObject(preferences.getString("records", "{}")).length() && calls.size() == 3, "Rejected plan changed journal or executed native operations");
  checks.put("journal persists before fake apply; partial retries survive recreation and target only pending original IDs");
 }

 private void requestRouting(SharedPreferences preferences) throws Exception {
  ArrayList<JSONObject> calls = new ArrayList<>();
  AndroidOwner owner = new AndroidOwner(test.getTargetContext(), preferences, request -> { calls.add(request); return new JSONObject(); }, command -> {}, JSONArray::new);
  JSONObject operation = new JSONObject().put("op", "revoke").put("peer", PEER).put("expected_revision", 7);
  owner.request(new JSONObject().put("op", "mutate").put("operation", operation));
  require("local_owner_mutate".equals(calls.get(0).getString("op")) && calls.get(0).getJSONObject("operation").getLong("expected_revision") == 7, "Owner mutation lost its revision");
  owner.request(new JSONObject().put("op", "invite_qr")); require("local_owner_invite".equals(calls.get(1).getString("op")), "Invitation routing incorrect");
  rejected(() -> owner.request(new JSONObject().put("op", "configure_public_gateway")));
  rejected(() -> owner.request(new JSONObject().put("op", "mutate").put("operation", operation).put("unexpected", true)));
  require(calls.size() == 2, "Unsupported request reached native code");
  checks.put("owner requests retain revision and use a fixed operation allowlist");
 }
 private void missingAcknowledgement(SharedPreferences preferences) throws Exception {
  ArrayList<Runnable> queued = new ArrayList<>(); JSONArray selected = new JSONArray().put(target(OWN, "This device"));
  AndroidOwner owner = new AndroidOwner(test.getTargetContext(), preferences,
    request -> new JSONObject().put("host", request.getString("host")).put("id", request.getString("id")).put("state", "applied").put("revision", JSONObject.NULL),
    queued::add, () -> selected);
  JSONObject started = (JSONObject)owner.data(new JSONObject().put("op", "start").put("plan", new JSONObject()
    .put("action", new JSONObject().put("op", "revoke").put("peer", PEER)).put("targets", selected)));
  queued.remove(0).run();
  JSONArray operations = (JSONArray)owner.data(new JSONObject().put("op", "operations")); boolean pending = false;
  for (int index=0;index<operations.length();index++) {
   JSONObject record = operations.getJSONObject(index);
   if (started.getString("id").equals(record.getString("id")))
    pending = "pending".equals(record.getJSONArray("targets").getJSONObject(0).getString("state"));
  }
  require(pending, "Applied receipt without durable revision was acknowledged");
  owner.data(new JSONObject().put("op", "retry").put("id", started.getString("id")));
  require(queued.size()==1, "Unconfirmed receipt was not retryable"); queued.remove(0).run();
  checks.put("an applied acknowledgement without a revision stays pending and retryable");
 }
 private static JSONObject service(String id, String kind) throws Exception { return new JSONObject().put("id", id).put("kind", kind).put("incarnation", "f".repeat(64)); }
 private static JSONObject target(String host, String access) throws Exception { return new JSONObject().put("host", host).put("name", "test-host").put("owner_access", access); }
 private interface Check { void run() throws Exception; }
 private static void rejected(Check check) throws Exception { boolean rejected = false; try { check.run(); } catch (Exception expected) { rejected = true; } require(rejected, "Invalid owner input accepted"); }
 private static void require(boolean value, String message) throws Exception { if (!value) throw new Exception(message); }
}
