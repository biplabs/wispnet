package com.biplabs.wispnet;

import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.res.AssetFileDescriptor;
import android.net.Uri;
import android.os.CancellationSignal;
import android.os.ParcelFileDescriptor;
import android.os.SystemClock;
import android.system.Os;
import android.system.OsConstants;
import android.system.StructStat;
import java.io.Closeable;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import org.json.JSONArray;
import org.json.JSONObject;

/** File-provider access and transfer jobs live in the foreground network process. */
final class AndroidFiles implements AutoCloseable {
 private static final int MAX_JOBS = 64;
 private static final long HISTORY_MS = 60 * 60 * 1000;
 private final Context context;
 private final ExecutorService work = Executors.newFixedThreadPool(2);
 private final Map<String, Job> jobs = new LinkedHashMap<>();
 private volatile boolean closed, stopped = true;

 private static final class Job {
  final String id = UUID.randomUUID().toString().replace("-", "")
      + UUID.randomUUID().toString().replace("-", "");
  final JSONObject request;
  final String source;
  final long startedAt = SystemClock.elapsedRealtime();
  final CancellationSignal cancellation = new CancellationSignal();
  JSONObject status;
  volatile boolean cancelled, started;
  volatile String nativeId;
  volatile Closeable sourceHandle;
  long finishedAt;

  Job(JSONObject request, String source) throws Exception {
   this.request = request;
   this.source = source;
   status = new JSONObject().put("transfer", id).put("state", "running")
       .put("phase", "queued").put("verified_bytes", 0).put("size_bytes", 0);
  }
 }

 AndroidFiles(Context context) {
  this.context = context.getApplicationContext();
  // A process-death interruption retains its native resume journal, not an
  // incomplete provider copy. Reopening the same URI can recreate that source.
  work.execute(() -> {
   File[] orphaned = new File(this.context.getCacheDir(), "wispnet-file-sources").listFiles();
   if (orphaned != null) for (File file : orphaned) {
    String name = file.getName();
    if (!name.matches("[0-9a-f]{64}\\.partial")) continue;
    synchronized (jobs) { if (jobs.containsKey(name.substring(0, 64))) continue; }
    file.delete();
   }
  });
 }

 JSONObject workspaces(JSONObject args) throws Exception {
  JSONObject target = target(args);
  authorize(target, null);
  return Native.request(target.put("op", "file_workspaces"));
 }

 JSONObject send(JSONObject args) throws Exception {
  if (closed || stopped) throw new Exception("Networking stopped. Go online and try again.");
  String source = FileTransferRules.document(args.getString("local_path"));
  Uri uri = Uri.parse(source);
  if (context.checkUriPermission(uri, android.os.Process.myPid(), android.os.Process.myUid(),
      Intent.FLAG_GRANT_READ_URI_PERMISSION) != PackageManager.PERMISSION_GRANTED)
   throw new Exception("File access expired. Select the file again using Choose files.");
  JSONObject request = target(args).put("op", "file_put")
      .put("workspace", FileTransferRules.uploadFolder(!args.has("workspace") ? "" :
          args.opt("workspace") instanceof String ? args.getString("workspace") : null))
      .put("remote_path", FileTransferRules.filename(args.getString("remote_path")))
      .put("source_id", source);
  if (args.has("resume") && !(args.get("resume") instanceof Boolean))
   throw new Exception("Resume must be true or false");
  request.put("resume", args.optBoolean("resume", true));
  Job job = new Job(request, source);
  synchronized (jobs) {
   if (closed || stopped) throw new Exception("Networking stopped. Go online and try again.");
   prune();
   if (jobs.size() >= MAX_JOBS) throw new Exception("The transfer queue is full. Wait for a file to finish.");
   jobs.put(job.id, job);
   work.execute(() -> run(job));
  }
  return snapshot(job);
 }

 JSONObject status(String id) throws Exception { return snapshot(find(id)); }

 JSONObject cancel(String id) throws Exception {
  Job job = find(id);
  synchronized (job) {
   if (finished(job.status)) return snapshot(job);
   job.cancelled = true;
   if (!job.started) fail(job, "CANCELLED", "Transfer cancelled", true);
  }
  try { job.cancellation.cancel(); }
  finally { closeQuietly(job.sourceHandle); }
  return snapshot(job);
 }

 JSONObject list() throws Exception {
  JSONArray values = new JSONArray();
  synchronized (jobs) {
   for (Job job : jobs.values()) values.put(snapshot(job));
  }
  return new JSONObject().put("transfers", values);
 }

 void cancelAll() {
  ArrayList<Job> copy;
  synchronized (jobs) { copy = new ArrayList<>(jobs.values()); }
  for (Job job : copy) try { cancel(job.id); } catch (Exception ignored) {}
 }

 void cancelHost(String host) {
  ArrayList<Job> copy;
  synchronized (jobs) { copy = new ArrayList<>(jobs.values()); }
  for (Job job : copy) if (host.equals(job.request.optString("host")))
   try { cancel(job.id); } catch (Exception ignored) {}
 }

 void online() { if (!closed) stopped = false; }
 void stop() { stopped = true; cancelAll(); }

 private JSONObject target(JSONObject args) throws Exception {
  String host = args.getString("host"), incarnation = args.getString("incarnation");
  if (!host.matches("[0-9a-f]{64}") || !incarnation.matches("[0-9a-f]{64}"))
   throw new Exception("Invalid Files target. Refresh this device.");
  return new JSONObject().put("host", host).put("service", "files").put("incarnation", incarnation);
 }

 private void authorize(JSONObject target, CancellationSignal cancellation) throws Exception {
  String host = target.getString("host"), incarnation = target.getString("incarnation");
  JSONObject catalog;
  long deadline = SystemClock.elapsedRealtime() + 15000;
  while (true) {
   if (cancellation != null) cancellation.throwIfCanceled();
   try {
    catalog = Native.request(new JSONObject().put("op", "catalog").put("host", host));
    break;
   } catch (Native.Failure busy) {
    // The native catalog baseline permits one refresh per peer at a time.
    // Concurrent uploads and the UI's refresh share that gate.
    if (busy.getMessage() == null || !busy.getMessage().contains("Catalog refresh busy")
        || SystemClock.elapsedRealtime() >= deadline) throw busy;
    Thread.sleep(50);
   }
  }
  if (!host.equals(catalog.optString("host"))) throw new Exception("Wrong Files host");
  JSONArray services = catalog.getJSONArray("services");
  boolean allowed = false;
  for (int n = 0; n < services.length(); n++) {
   JSONObject service = services.getJSONObject(n);
   if ("files".equals(service.optString("id")) && "files".equals(service.optString("kind"))
       && incarnation.equals(service.optString("incarnation"))) allowed = true;
  }
  if (!allowed) throw new Exception("Files access changed or is not permitted. Refresh this device.");
 }

 private void run(Job job) {
  File staged = null;
  try {
   synchronized (job) {
    if (job.cancelled) return;
    job.started = true;
    job.status.put("phase", "preparing");
   }
   // Admission returns a cancellable queue handle immediately. Perform remote
   // authorization in a worker, before opening or copying the selected source.
   checkCancelled(job);
   authorize(job.request, job.cancellation);
   checkCancelled(job);
   JSONObject nativeJob;
   Uri uri = Uri.parse(job.source);
   try (AssetFileDescriptor asset = context.getContentResolver()
       .openAssetFileDescriptor(uri, "r", job.cancellation)) {
    if (asset == null) throw new Exception("The selected file is unavailable. Select it again.");
    ParcelFileDescriptor descriptor = asset.getParcelFileDescriptor();
    job.sourceHandle = descriptor;
    checkCancelled(job);
    StructStat metadata = Os.fstat(descriptor.getFileDescriptor());
    if (OsConstants.S_ISDIR(metadata.st_mode)) throw new Exception("Choose files, not folders.");
    boolean direct = asset.getStartOffset() == 0 && OsConstants.S_ISREG(metadata.st_mode)
        && (asset.getDeclaredLength() < 0 || asset.getDeclaredLength() == metadata.st_size);
    if (direct) {
     try { Os.lseek(descriptor.getFileDescriptor(), 0, OsConstants.SEEK_SET); }
     catch (android.system.ErrnoException pipe) { direct = false; }
    }
    if (direct) {
     checkCancelled(job);
     nativeJob = Native.requestFileDescriptor(job.request, descriptor);
    } else {
     File directory = new File(context.getCacheDir(), "wispnet-file-sources");
     if (!directory.isDirectory() && !directory.mkdir())
      throw new Exception("Cannot prepare this provider's file. Free some storage and retry.");
     Os.chmod(directory.getAbsolutePath(), 0700);
     staged = new File(directory, job.id + ".partial");
     long size = asset.getDeclaredLength();
     if (size > 0 && size > directory.getUsableSpace())
      throw new Exception("Not enough free storage to prepare this provider's file. Save a local copy or free some storage first.");
     synchronized (job) {
      job.status.put("phase", "copying").put("copied_bytes", 0).put("size_bytes", Math.max(0, size));
     }
     try (InputStream input = asset.createInputStream(); FileOutputStream output = new FileOutputStream(staged)) {
      job.sourceHandle = input;
      Os.chmod(staged.getAbsolutePath(), 0600);
      byte[] buffer = new byte[256 * 1024];
      long copied = 0;
      while (true) {
       checkCancelled(job);
       int read = input.read(buffer);
       if (read < 0) break;
       output.write(buffer, 0, read);
       copied += read;
       synchronized (job) { job.status.put("copied_bytes", copied); }
      }
     }
     checkCancelled(job);
     try (ParcelFileDescriptor prepared = ParcelFileDescriptor.open(staged, ParcelFileDescriptor.MODE_READ_ONLY)) {
      job.sourceHandle = prepared;
      nativeJob = Native.requestFileDescriptor(job.request, prepared);
     }
    }
   } finally { job.sourceHandle = null; }
   job.nativeId = nativeJob.getString("transfer");
   update(job, nativeJob);
   boolean cancellationSent = false;
   while (!finished(nativeJob)) {
    try {
     if ((job.cancelled || closed) && !cancellationSent) {
      nativeJob = Native.request(new JSONObject().put("op", "file_cancel").put("transfer", job.nativeId));
      update(job, nativeJob);
      cancellationSent = true;
     }
     if (finished(nativeJob)) break;
     nativeJob = Native.request(new JSONObject().put("op", "file_status").put("transfer", job.nativeId));
     update(job, nativeJob);
    } catch (Exception unavailable) {
     if (unavailable.getMessage() != null && unavailable.getMessage().contains("handle unavailable")) throw unavailable;
     synchronized (job) {
      job.status.put("error", new JSONObject().put("code", "UNAVAILABLE")
          .put("message", job.cancelled ? "Waiting to confirm cancellation…" : "Waiting for the transfer service…"));
     }
    }
    if (!finished(nativeJob)) Thread.sleep(500);
   }
  } catch (Exception failure) {
   if (job.nativeId != null && job.cancelled) {
    try {
     JSONObject nativeJob = Native.request(new JSONObject().put("op", "file_cancel").put("transfer", job.nativeId));
     if (finished(nativeJob)) { update(job, nativeJob); return; }
    } catch (Exception ignored) {}
   }
   synchronized (job) {
    fail(job, job.cancelled || closed ? "CANCELLED" : failure instanceof Native.Failure
        ? ((Native.Failure)failure).code : "UNAVAILABLE",
        job.cancelled || closed ? "Transfer cancelled" : failure.getMessage() == null
        ? "Cannot read this file. Select it again or save a local copy first." : failure.getMessage(),
        job.cancelled || closed);
   }
  } finally {
   closeQuietly(job.sourceHandle);
   job.sourceHandle = null;
   if (staged != null) staged.delete();
  }
 }

 private static void checkCancelled(Job job) throws Exception {
  if (job.cancelled || Thread.currentThread().isInterrupted()) throw new Exception("Transfer cancelled");
 }

 private static boolean finished(JSONObject status) {
  String state = status.optString("state");
  return "complete".equals(state) || "failed".equals(state) || "cancelled".equals(state);
 }

 private static void update(Job job, JSONObject status) throws Exception {
  synchronized (job) {
   long copied = job.status.optLong("copied_bytes", 0);
   job.status = new JSONObject(status.toString()).put("transfer", job.id);
   if (copied > 0) job.status.put("copied_bytes", copied);
   if (finished(status)) job.finishedAt = SystemClock.elapsedRealtime();
  }
 }

 private static void fail(Job job, String code, String message, boolean cancelled) {
  try {
   job.status.put("state", cancelled ? "cancelled" : "failed")
       .put("error", new JSONObject().put("code", code).put("message", message));
   job.finishedAt = SystemClock.elapsedRealtime();
  } catch (org.json.JSONException impossible) { throw new IllegalStateException(impossible); }
 }

 private static JSONObject snapshot(Job job) throws Exception {
  synchronized (job) {
   JSONObject result = new JSONObject(job.status.toString());
   for (String key : new String[]{"host", "incarnation", "workspace", "remote_path"})
    result.put(key, job.request.get(key));
   return result.put("local_path", job.source).put("name", job.request.getString("remote_path"))
       .put("duration_ms", (job.finishedAt == 0 ? SystemClock.elapsedRealtime() : job.finishedAt) - job.startedAt);
  }
 }

 private Job find(String id) throws Exception {
  if (id == null || !id.matches("[0-9a-f]{64}")) throw new Exception("Invalid file transfer handle");
  synchronized (jobs) {
   Job job = jobs.get(id);
   if (job == null) throw new Exception("File transfer handle unavailable; select the file again to resume");
   return job;
  }
 }

 private void prune() {
  long now = SystemClock.elapsedRealtime();
  Iterator<Job> iterator = jobs.values().iterator();
  while (iterator.hasNext()) {
   Job job = iterator.next();
   synchronized (job) { if (job.finishedAt != 0 && now - job.finishedAt > HISTORY_MS) iterator.remove(); }
  }
  if (jobs.size() >= MAX_JOBS) {
   iterator = jobs.values().iterator();
   while (iterator.hasNext()) {
    Job job = iterator.next();
    synchronized (job) { if (job.finishedAt != 0) { iterator.remove(); break; } }
   }
  }
 }

 private static void closeQuietly(Closeable source) { if (source != null) try { source.close(); } catch (Exception ignored) {} }

 @Override public void close() { closed = true; cancelAll(); work.shutdownNow(); }
}
