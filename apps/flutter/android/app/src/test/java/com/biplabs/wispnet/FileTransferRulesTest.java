package com.biplabs.wispnet;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import org.junit.Test;

public final class FileTransferRulesTest {
 @Test public void onlyGrantedDocumentReferencesCanReachTheAdapter() throws Exception {
  String document = "content://com.android.providers.media.documents/document/video%3A42";
  assertEquals(document, FileTransferRules.document(document));
  for (String invalid : new String[]{"/storage/emulated/0/private.bin", "file:///etc/passwd",
      "https://example.invalid/model.bin", "content:/document/42", "content://", "content://provider/42#other",
      "content://provider/" + "a".repeat(2048)})
   assertThrows(Exception.class, () -> FileTransferRules.document(invalid));
 }

 @Test public void originalUnicodeNamesArePreservedAndBoundedByUtf8Bytes() throws Exception {
  assertEquals("model weights 🧪.bin", FileTransferRules.filename("model weights 🧪.bin"));
  assertEquals("x".repeat(255), FileTransferRules.filename("x".repeat(255)));
  assertThrows(Exception.class, () -> FileTransferRules.filename("x".repeat(256)));
  assertThrows(Exception.class, () -> FileTransferRules.filename("界".repeat(86)));
 }

 @Test public void providerNamesCannotEscapeTheReceivingFolderOrTouchManifests() {
  for (String invalid : new String[]{"", ".", "..", "../secret", "/absolute", "folder/file",
      "folder\\file", "bad\0name", ".wispnet-manifest"})
   assertThrows(Exception.class, () -> FileTransferRules.filename(invalid));
 }

 @Test public void receivingFolderMustBeAnExportName() throws Exception {
  assertEquals("training.v1", FileTransferRules.folder("training.v1"));
  for (String invalid : new String[]{"", "../incoming", "/incoming", "Incoming", ".hidden", "a".repeat(65)})
   assertThrows(Exception.class, () -> FileTransferRules.folder(invalid));
 }

 @Test public void uploadsCanUseTheReceiversDefaultWithoutKnowingItsName() throws Exception {
  assertEquals("", FileTransferRules.uploadFolder(""));
  assertEquals("training.v1", FileTransferRules.uploadFolder("training.v1"));
  assertThrows(Exception.class, () -> FileTransferRules.uploadFolder(null));
  for (String invalid : new String[]{"../incoming", "/incoming", "Incoming", ".hidden", "a".repeat(65)})
   assertThrows(Exception.class, () -> FileTransferRules.uploadFolder(invalid));
 }
}
