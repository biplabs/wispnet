package com.biplabs.wispnet;

import android.app.Activity;
import android.content.ClipData;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import java.util.ArrayList;

/** Test-only sender: exercises real cross-UID URI grants and the system Sharesheet. */
public final class ShareTestActivity extends Activity {
 @Override public void onCreate(Bundle state) {
  super.onCreate(state);
  int count = getIntent().getIntExtra("count", 1);
  if (count < 1 || count > 65) throw new IllegalArgumentException("Invalid fixture count");
  ArrayList<Uri> sources = new ArrayList<>();
  for (int n = 0; n < count; n++) sources.add(FilesTestProvider.fixture("direct", 4096 + n, getIntent().getIntExtra("seed", 173) + n));
  Intent share = new Intent(count == 1 ? Intent.ACTION_SEND : Intent.ACTION_SEND_MULTIPLE)
      .setType(getIntent().getStringExtra("mime") == null ? "application/octet-stream" : getIntent().getStringExtra("mime"))
      .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
  ClipData clip = ClipData.newRawUri("Wispnet share test", sources.get(0));
  for (int n = 1; n < sources.size(); n++) clip.addItem(new ClipData.Item(sources.get(n)));
  share.setClipData(clip);
  if (count == 1) share.putExtra(Intent.EXTRA_STREAM, sources.get(0));
  else share.putParcelableArrayListExtra(Intent.EXTRA_STREAM, sources);
  if (getIntent().getBooleanExtra("direct", false)) {
   share.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP);
   share.setClassName("app.wispnet.android", "com.biplabs.wispnet.FlutterMainActivity");
   startActivity(share);
  } else startActivity(Intent.createChooser(share, "Share test files"));
 }
}
