package com.biplabs.wispnet;

import java.net.URI;
import java.nio.charset.StandardCharsets;

/** Names and document references accepted by the Android file picker/adapter. */
final class FileTransferRules {
 static final int MAX_SELECTION = 64;

 static String document(String value) throws Exception {
  if (value == null || value.getBytes(StandardCharsets.UTF_8).length > 2048)
   throw new Exception("Select the file again using Choose files");
  URI uri = new URI(value);
  if (!"content".equals(uri.getScheme()) || uri.getRawAuthority() == null
      || uri.getRawAuthority().isEmpty() || uri.getRawFragment() != null)
   throw new Exception("Choose files using the Android file picker");
  return value;
 }

 static String filename(String value) throws Exception {
  if (value == null || value.isEmpty() || value.equals(".") || value.equals("..")
      || value.startsWith(".wispnet-") || value.indexOf('/') >= 0
      || value.indexOf('\\') >= 0 || value.indexOf('\0') >= 0
      || value.getBytes(StandardCharsets.UTF_8).length > 255)
   throw new Exception("This file name cannot be sent. Rename the file and select it again.");
  return value;
 }

 static String folder(String value) throws Exception {
  if (value == null || !value.matches("[a-z0-9][a-z0-9._-]{0,63}"))
   throw new Exception("Choose a receiving folder");
  return value;
 }

 static String uploadFolder(String value) throws Exception {
  // The receiver selects its configured default; explicit SDK targets remain valid.
  return "".equals(value) ? "" : folder(value);
 }

 private FileTransferRules() {}
}
