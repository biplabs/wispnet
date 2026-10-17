package com.biplabs.wispnet;
import android.os.Bundle;
import android.view.WindowManager;
/** Captured capabilities must not appear in screenshots or recents. */
public final class PairingCaptureActivity extends com.journeyapps.barcodescanner.CaptureActivity {
 @Override public void onCreate(Bundle state){super.onCreate(state);getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);}
}
