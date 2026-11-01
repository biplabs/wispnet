package com.biplabs.wispnet;

import android.content.Context;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.os.SystemClock;
import android.system.Os;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import org.json.JSONArray;
import org.json.JSONObject;

/** Pulls only requested files, then publishes verified data to a locally chosen SAF folder. */
final class AndroidDownloads implements AutoCloseable {
 private final Context context;
 private final ExecutorService work = Executors.newFixedThreadPool(2);
 private final Map<String, Job> jobs = new LinkedHashMap<>();
 private volatile boolean closed, stopped = true;

 private static final class Job {
  final String id = UUID.randomUUID().toString().replace("-", "") + UUID.randomUUID().toString().replace("-", "");
  final JSONObject request;
  final String root, name, binding;
  final File staged;
  JSONObject status;
  volatile String nativeId;
  volatile boolean cancelled;
  volatile long finishedAt;
  Job(JSONObject request, String root, String name, String binding, File staged) throws Exception {
   this.request = request; this.root = root; this.name = name; this.binding = binding; this.staged = staged;
   status = new JSONObject().put("transfer", id).put("state", "running").put("phase", "queued")
       .put("verified_bytes", 0).put("size_bytes", 0);
  }
 }

 AndroidDownloads(Context context) { this.context = context.getApplicationContext(); }

 JSONObject prepare(Uri tree) throws Exception {
  JSONObject folder = new AndroidReceiving(context).prepare(tree);
  return new JSONObject().put("path", folder.getString("folder")).put("label", folder.getString("label"));
 }

 JSONObject list(JSONObject args) throws Exception {
  if (closed || stopped) throw new Exception("Go online before browsing shared files");
  JSONObject request = target(args).put("op", "file_list").put("path", args.optString("path", ""));
  if (args.has("cursor")) request.put("cursor", args.get("cursor"));
  return Native.request(request);
 }

 JSONObject get(JSONObject args) throws Exception {
  if (closed || stopped) throw new Exception("Go online before downloading files");
  String root = args.getString("local_path"), name = FileTransferRules.filename(args.getString("name"));
  // A destination token only names a folder previously selected by this device's owner.
  String record = context.getSharedPreferences("receiving-folders", 0).getString(root, null);
  if (record == null) throw new SecurityException("Choose a download folder first");
  new AndroidReceiving(context).prepare(Uri.parse(new JSONObject(record).getString("uri")));
  JSONObject request = target(args).put("op", "file_get").put("path", args.getString("path"));
  if (args.has("resume") && !(args.get("resume") instanceof Boolean)) throw new Exception("Resume must be true or false");
  request.put("resume", args.optBoolean("resume", true));
  String binding = hash(new JSONArray().put(root).put(name).put(request.getString("host"))
      .put(request.getString("service")).put(request.getString("incarnation")).put(request.getString("path")).toString());
  File directory = new File(context.getFilesDir(), "downloads/" + binding);
  if (!directory.isDirectory() && !directory.mkdirs()) throw new Exception("Cannot prepare download storage");
  Os.chmod(directory.getAbsolutePath(), 0700);
  File staged = new File(directory, "verified-file");
  request.put("local_path", staged.getCanonicalPath());
  Job job = new Job(request, root, name, binding, staged);
  synchronized (jobs) {
   if (closed || stopped) throw new Exception("Networking stopped. Go online and try again.");
   long cutoff = SystemClock.elapsedRealtime() - 60 * 60 * 1000;
   jobs.entrySet().removeIf(entry -> entry.getValue().finishedAt > 0 && entry.getValue().finishedAt < cutoff);
   for (Job existing : jobs.values()) synchronized (existing) {
    if (existing.binding.equals(binding) && !finished(existing.status)) throw new Exception("This download is already active");
   }
   if (jobs.size() >= 64) throw new Exception("The transfer queue is full. Wait for a file to finish.");
   jobs.put(job.id, job);
   work.execute(() -> run(job));
  }
  return snapshot(job);
 }

 boolean contains(String id) { synchronized (jobs) { return jobs.containsKey(id); } }
 JSONArray snapshots() throws Exception {
  JSONArray result = new JSONArray();
  synchronized (jobs) {
   for (Job job : jobs.values()) {
    JSONObject request = target(job.request).put("path", job.request.getString("path"))
        .put("local_path", job.root).put("name", job.name).put("resume", true);
    result.put(snapshot(job).put("request", request).put("name", job.name));
   }
  }
  return result;
 }
 JSONObject status(String id) throws Exception { return snapshot(find(id)); }
 JSONObject cancel(String id) throws Exception {
  Job job = find(id);
  synchronized (job) { if (!finished(job.status)) job.cancelled = true; }
  return snapshot(job);
 }
 void cancelHost(String host) {
  ArrayList<Job> copy;
  synchronized (jobs) { copy = new ArrayList<>(jobs.values()); }
  for (Job job : copy) if (host == null || host.equals(job.request.optString("host")))
   try { cancel(job.id); } catch (Exception ignored) {}
 }
 void online() { if (!closed) stopped = false; }
 void stop() { stopped = true; cancelHost(null); }
 @Override public void close() { closed = true; stop(); work.shutdown(); }

 private void run(Job job) {
  try {
   check(job);
   // This private artifact belongs only to this source/destination binding.
   // An explicit restart may discard it; provider documents are never replaced.
   if (!job.request.optBoolean("resume", true)
       && (job.staged.exists() || java.nio.file.Files.isSymbolicLink(job.staged.toPath()))
       && !job.staged.delete()) throw new Exception("Cannot discard the previous staged download");
   JSONObject nativeStatus = Native.request(job.request);
   job.nativeId = nativeStatus.getString("transfer");
   while (true) {
    update(job, nativeStatus);
    if (finished(nativeStatus)) break;
    if (job.cancelled || closed || stopped)
     Native.request(new JSONObject().put("op", "file_cancel").put("transfer", job.nativeId));
    Thread.sleep(300);
    nativeStatus = Native.request(new JSONObject().put("op", "file_status").put("transfer", job.nativeId));
   }
   if (!"complete".equals(nativeStatus.optString("state"))) {
    JSONObject error = nativeStatus.optJSONObject("error");
    if (error != null && "CONFLICT".equals(error.optString("code")) && job.staged.isFile())
     error.put("code", "STALE_VERSION").put("message", "The source changed after download. Restart to discard the previous staged copy.");
    synchronized (job) { job.status = new JSONObject(nativeStatus.toString()).put("transfer", job.id); }
    return;
   }
   check(job);
   JSONObject file = nativeStatus.getJSONObject("file");
   String receipt = hash(new JSONArray().put(job.binding).put(file.getString("sha256")).toString());
   synchronized (job) { job.status.put("phase", "saving"); }
   try (ParcelFileDescriptor source = ParcelFileDescriptor.open(job.staged, ParcelFileDescriptor.MODE_READ_ONLY)) {
    String result = new AndroidReceiving(context).publish(new JSONObject().put("transfer", receipt)
        .put("root", job.root).put("path", job.name).put("sha256", file.getString("sha256"))
        .put("size_bytes", file.getLong("size_bytes")), source.getFd(), () -> job.cancelled || closed || stopped);
    if (!"ok".equals(result)) throw new Native.Failure(
        "conflict".equals(result) ? "A file with this name already exists in the download folder" : "Cannot save this download",
        "conflict".equals(result) ? "CONFLICT" : "UNAVAILABLE");
   }
   synchronized (job) { job.status.put("state", "complete").put("phase", "complete").put("file", file); }
   job.staged.delete();
  } catch (Exception failure) {
   if (job.nativeId != null) try {
    Native.request(new JSONObject().put("op", "file_cancel").put("transfer", job.nativeId));
   } catch (Exception ignored) {}
   synchronized (job) {
    boolean cancelled = job.cancelled || closed || stopped;
    try { job.status.put("state", cancelled ? "cancelled" : "failed")
        .put("error", new JSONObject().put("code", cancelled ? "CANCELLED" : failure instanceof Native.Failure
            ? ((Native.Failure)failure).code : "UNAVAILABLE")
            .put("message", cancelled ? "Download paused. Select the same file and folder to resume."
                : failure.getMessage() == null ? "Download interrupted. Try again to resume." : failure.getMessage()));
    } catch (org.json.JSONException impossible) { throw new IllegalStateException(impossible); }
   }
  } finally { synchronized (job) { job.finishedAt = SystemClock.elapsedRealtime(); } }
 }

 private static void update(Job job, JSONObject status) throws Exception {
  synchronized (job) {
   job.status = new JSONObject(status.toString()).put("transfer", job.id);
   // Completion includes publication into the chosen provider, not just private staging.
   if ("complete".equals(job.status.optString("state"))) job.status.put("state", "running").put("phase", "saving");
  }
 }
 private void check(Job job) throws Exception {
  if (job.cancelled || closed || stopped) throw new Exception("Download paused");
 }
 private Job find(String id) throws Exception {
  synchronized (jobs) { Job job = jobs.get(id); if (job != null) return job; }
  throw new Exception("Download handle unavailable. Select the same file and folder to resume.");
 }
 private static boolean finished(JSONObject status) {
  return java.util.Set.of("complete", "failed", "cancelled").contains(status.optString("state"));
 }
 private static JSONObject snapshot(Job job) throws Exception { synchronized (job) { return new JSONObject(job.status.toString()); } }
 private static JSONObject target(JSONObject args) throws Exception {
  String host = args.getString("host"), service = args.getString("service"), incarnation = args.getString("incarnation");
  if (!host.matches("[0-9a-f]{64}") || !incarnation.matches("[0-9a-f]{64}")
      || !service.matches("[a-z0-9][a-z0-9._-]{0,63}")) throw new Exception("Invalid shared files target. Refresh this device.");
  return new JSONObject().put("host", host).put("service", service).put("incarnation", incarnation);
 }
 private static String hash(String value) throws Exception {
  byte[] bytes = MessageDigest.getInstance("SHA-256").digest(value.getBytes(StandardCharsets.UTF_8));
  StringBuilder result = new StringBuilder();
  for (byte item : bytes) result.append(String.format(java.util.Locale.ROOT, "%02x", item & 255));
  return result.toString();
 }
}
