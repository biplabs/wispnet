package com.biplabs.wispnet;

import android.content.ClipData;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.net.Uri;
import android.os.CancellationSignal;
import android.provider.OpenableColumns;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.LinkedHashSet;
import org.json.JSONArray;
import org.json.JSONObject;

/** Reads document metadata only; networking and JNI remain in AccessService. */
final class AndroidFilePicker {
 static Intent intent() {
  return new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("*/*")
      .addCategory(Intent.CATEGORY_OPENABLE).putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
      .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
 }

 static JSONObject selection(Context context, Intent data, CancellationSignal cancellation) throws Exception {
  LinkedHashSet<Uri> sources = new LinkedHashSet<>();
  if (Intent.ACTION_SEND.equals(data.getAction())) {
   Uri stream = data.getParcelableExtra(Intent.EXTRA_STREAM, Uri.class);
   if (stream != null) sources.add(stream);
  } else if (Intent.ACTION_SEND_MULTIPLE.equals(data.getAction())) {
   java.util.ArrayList<Uri> streams = data.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri.class);
   if (streams != null) {
    if (streams.size() > FileTransferRules.MAX_SELECTION) throw new Exception("Choose up to 64 files at a time.");
    sources.addAll(streams);
   }
  }
  if (data.getData() != null) sources.add(data.getData());
  ClipData clip = data.getClipData();
  if (clip != null) {
   if (clip.getItemCount() > FileTransferRules.MAX_SELECTION)
    throw new Exception("Choose up to 64 files at a time.");
   for (int n = 0; n < clip.getItemCount(); n++) {
    Uri uri = clip.getItemAt(n).getUri();
    if (uri != null) sources.add(uri);
   }
  }
  if (sources.size() > FileTransferRules.MAX_SELECTION) throw new Exception("Choose up to 64 files at a time.");
  JSONArray files = new JSONArray();
  for (Uri uri : sources) {
   cancellation.throwIfCanceled();
   if (uri == null) throw new Exception("Cannot read a shared file.");
   FileTransferRules.document(uri.toString());
   if (context.checkUriPermission(uri, android.os.Process.myPid(), android.os.Process.myUid(),
       Intent.FLAG_GRANT_READ_URI_PERMISSION) != PackageManager.PERMISSION_GRANTED)
    throw new Exception("Android did not grant access to a selected file. Select it again.");
   String name = null;
   long size = 0;
   try (Cursor cursor = context.getContentResolver().query(uri,
       new String[]{OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE}, null, null, null, cancellation)) {
    if (cursor != null && cursor.moveToFirst()) {
     int nameColumn = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
     int sizeColumn = cursor.getColumnIndex(OpenableColumns.SIZE);
     if (nameColumn >= 0 && !cursor.isNull(nameColumn)) name = cursor.getString(nameColumn);
     if (sizeColumn >= 0 && !cursor.isNull(sizeColumn)) size = Math.max(0, cursor.getLong(sizeColumn));
    }
   }
   if (name == null || name.isEmpty()) {
    byte[] hash = MessageDigest.getInstance("SHA-256").digest(uri.toString().getBytes(StandardCharsets.UTF_8));
    StringBuilder suffix = new StringBuilder();
    for (int n = 0; n < 6; n++) suffix.append(String.format(java.util.Locale.ROOT, "%02x", hash[n] & 255));
    name = "selected-file-" + suffix;
   }
   files.put(new JSONObject().put("path", uri.toString())
       .put("name", FileTransferRules.filename(name)).put("size_bytes", size));
  }
  // Transient provider grants also work while the app is alive. Persist only
  // when offered by the picker; providers are not required to support it.
  if ((data.getFlags() & Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION) != 0) {
   for (Uri uri : sources) {
    cancellation.throwIfCanceled();
    try { context.getContentResolver().takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION); }
    catch (SecurityException transientOnly) {}
   }
  }
  return new JSONObject().put("files", files);
 }

 private AndroidFilePicker() {}
}
