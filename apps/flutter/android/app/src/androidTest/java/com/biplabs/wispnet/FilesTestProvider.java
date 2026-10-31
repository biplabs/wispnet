package com.biplabs.wispnet;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Intent;
import android.content.res.AssetFileDescriptor;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.Binder;
import android.os.Bundle;
import android.os.CancellationSignal;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import java.io.File;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.security.MessageDigest;
import java.util.List;

/** Generated public fixtures in the test APK, never application/user documents. */
public final class FilesTestProvider extends ContentProvider {
 public static final String AUTHORITY = "app.wispnet.android.test.files";
 private static final int CHUNK = 256 * 1024;
 private static final int PREFIX = 37, SUFFIX = 29;

 @Override public boolean onCreate() { return true; }

 public static Uri fixture(String kind, long size, int seed) {
  return Uri.parse("content://" + AUTHORITY + "/" + kind + "/" + size + "/" + seed);
 }

 private static final class Fixture {
  final String kind;
  final long size;
  final int seed;
  Fixture(Uri uri) throws FileNotFoundException {
   List<String> parts = uri.getPathSegments();
   try {
    if (!AUTHORITY.equals(uri.getAuthority()) || parts.size() != 3) throw new Exception();
    kind = parts.get(0); size = Long.parseLong(parts.get(1)); seed = Integer.parseInt(parts.get(2));
    if (!(kind.equals("direct") || kind.equals("pipe") || kind.equals("slow") || kind.equals("slice"))
        || size < 0 || size > 512L * 1024 * 1024 || seed < 0 || seed > 255) throw new Exception();
   } catch (Exception invalid) { throw new FileNotFoundException("Invalid generated fixture"); }
  }
  String name() { return "fixture-" + kind + "-" + size + "-" + seed + ".bin"; }
 }

 @Override public Cursor query(Uri uri, String[] projection, String selection, String[] arguments, String order) {
  try {
   Fixture source = new Fixture(uri);
   String[] columns = projection == null ? new String[]{OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE} : projection;
   MatrixCursor cursor = new MatrixCursor(columns); Object[] row = new Object[columns.length];
   for (int n = 0; n < columns.length; n++) {
    if (OpenableColumns.DISPLAY_NAME.equals(columns[n])) row[n] = source.name();
    else if (OpenableColumns.SIZE.equals(columns[n])) row[n] = source.size;
   }
   cursor.addRow(row); return cursor;
  } catch (FileNotFoundException invalid) { throw new IllegalArgumentException(invalid); }
 }
 @Override public String getType(Uri uri) { return "application/octet-stream"; }
 @Override public Uri insert(Uri uri, ContentValues values) { throw new UnsupportedOperationException(); }
 @Override public int update(Uri uri, ContentValues values, String where, String[] args) { throw new UnsupportedOperationException(); }
 @Override public int delete(Uri uri, String where, String[] args) { throw new UnsupportedOperationException(); }

 @Override public Bundle call(String method, String argument, Bundle extras) {
  if ("grant".equals(method)) {
   Uri uri = Uri.parse(argument);
   try { new Fixture(uri); } catch (FileNotFoundException invalid) { throw new IllegalArgumentException(invalid); }
   long identity = Binder.clearCallingIdentity();
   try { getContext().grantUriPermission("app.wispnet.android", uri, Intent.FLAG_GRANT_READ_URI_PERMISSION); }
   finally { Binder.restoreCallingIdentity(identity); }
   Bundle result = new Bundle(); result.putString("uri", uri.toString()); return result;
  }
  if ("clean".equals(method)) {
   File directory = new File(getContext().getCacheDir(), "generated-files");
   File[] files = directory.listFiles();
   if (files != null) for (File file : files) if (file.isFile()) file.delete();
   return Bundle.EMPTY;
  }
  throw new IllegalArgumentException("Unknown fixture operation");
 }

 @Override public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
  return openAssetFile(uri, mode, null).getParcelFileDescriptor();
 }
 @Override public ParcelFileDescriptor openFile(Uri uri, String mode, CancellationSignal signal) throws FileNotFoundException {
  return openAssetFile(uri, mode, signal).getParcelFileDescriptor();
 }
 @Override public AssetFileDescriptor openAssetFile(Uri uri, String mode) throws FileNotFoundException {
  return openAssetFile(uri, mode, null);
 }
 @Override public AssetFileDescriptor openAssetFile(Uri uri, String mode, CancellationSignal signal) throws FileNotFoundException {
  if (!"r".equals(mode)) throw new FileNotFoundException("Read-only fixtures");
  if (signal != null) signal.throwIfCanceled();
  Fixture source = new Fixture(uri);
  try {
   if (source.kind.equals("pipe") || source.kind.equals("slow")) {
    ParcelFileDescriptor[] pipe = ParcelFileDescriptor.createPipe();
    Thread writer = new Thread(() -> {
     try (OutputStream out = new ParcelFileDescriptor.AutoCloseOutputStream(pipe[1])) {
      writePattern(out, source.size, source.seed, source.kind.equals("slow") ? 20 : 0);
     } catch (Exception cancelled) { /* A cancelled reader closes the pipe. */ }
    }, "wispnet-generated-provider");
    writer.setDaemon(true); writer.start();
    return new AssetFileDescriptor(pipe[0], 0, AssetFileDescriptor.UNKNOWN_LENGTH);
   }
   File directory = new File(getContext().getCacheDir(), "generated-files");
   if (!directory.isDirectory() && !directory.mkdir()) throw new IOException("Cannot create fixture directory");
   File file = new File(directory, source.name());
   boolean slice = source.kind.equals("slice");
   long expected = source.size + (slice ? PREFIX + SUFFIX : 0);
   synchronized (FilesTestProvider.class) {
    if (!file.isFile() || file.length() != expected) {
     try (OutputStream out = new FileOutputStream(file)) {
      if (slice) out.write(new byte[PREFIX]);
      writePattern(out, source.size, source.seed, 0);
      if (slice) out.write(new byte[SUFFIX]);
     }
    }
   }
   return new AssetFileDescriptor(ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY), slice ? PREFIX : 0, source.size);
  } catch (IOException failure) { throw new FileNotFoundException(failure.toString()); }
 }

 private static byte[] pattern(int seed) {
  byte[] bytes = new byte[CHUNK];
  for (int n = 0; n < bytes.length; n++) bytes[n] = (byte)(n + seed);
  return bytes;
 }
 private static void writePattern(OutputStream out, long length, int seed, int delayMillis) throws IOException {
  byte[] bytes = pattern(seed);
  for (long offset = 0; offset < length;) {
   int count = (int)Math.min(bytes.length, length - offset); out.write(bytes, 0, count); offset += count;
   if (delayMillis > 0) try { Thread.sleep(delayMillis); } catch (InterruptedException cancelled) { Thread.currentThread().interrupt(); throw new IOException(cancelled); }
  }
 }
 public static String expectedSha256(long length, int seed) throws Exception {
  MessageDigest digest = MessageDigest.getInstance("SHA-256"); byte[] bytes = pattern(seed);
  for (long at = 0; at < length;) { int count = (int)Math.min(bytes.length, length - at); digest.update(bytes, 0, count); at += count; }
  StringBuilder hex = new StringBuilder(); for (byte value : digest.digest()) hex.append(String.format(java.util.Locale.ROOT, "%02x", value & 255));
  return hex.toString();
 }
}
