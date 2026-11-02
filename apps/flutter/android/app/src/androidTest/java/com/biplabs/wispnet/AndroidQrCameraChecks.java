package com.biplabs.wispnet;

import android.app.Activity;
import android.app.Instrumentation;
import android.os.Bundle;
import android.util.Size;

/** Bounds/lifecycle fixtures only: never opens a camera or requests permission. */
final class AndroidQrCameraChecks {
 static void run(Instrumentation test) {
  Bundle result = new Bundle();
  try {
   Size chosen = AndroidQrCamera.chooseSize(new Size[]{new Size(4096, 3000), new Size(640, 480), new Size(1280, 720)});
   if (!new Size(1280, 720).equals(chosen)) throw new AssertionError("Capture exceeded the bounded JPEG dimensions");
   if (AndroidQrCamera.chooseSize(new Size[]{new Size(4096, 3000)}) != null) throw new AssertionError("Oversized-only camera must not allocate a reader");
   int[] replies = {0};
   AndroidQrCamera camera = new AndroidQrCamera(test.getTargetContext());
   camera.frame((bytes, error) -> {
    if (bytes != null || error == null) throw new AssertionError("Capture before explicit camera start must fail");
    replies[0]++;
   });
   camera.close(); camera.close();
   if (replies[0] != 1) throw new AssertionError("Frame reply completed more than once");
   result.putString("camera", "bounded JPEG dimensions and explicit-start lifecycle passed; no camera opened");
   test.finish(Activity.RESULT_OK, result);
  } catch (Throwable failure) {
   result.putString("error", failure.toString()); test.finish(Activity.RESULT_CANCELED, result);
  }
 }
}
