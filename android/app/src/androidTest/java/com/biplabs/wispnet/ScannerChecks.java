package com.biplabs.wispnet;
import android.app.*;
import android.os.Bundle;
/** Launch the real capture Activity; simulated scan results cannot catch lifecycle crashes. */
final class ScannerChecks {
 static void run(AppTestRunner test)throws Exception {
  test.resume();
  Instrumentation.ActivityMonitor monitor=test.addMonitor(PairingCaptureActivity.class.getName(),null,false);
  try {
   test.click("Scan pairing QR");
   Activity capture=test.waitForMonitorWithTimeout(monitor,10000);
   if(capture==null)throw new Exception("Scanner did not launch");
   test.await(()->{final boolean[] active={false};test.runOnMainSync(()->{
    com.journeyapps.barcodescanner.DecoratedBarcodeView view=capture.findViewById(com.google.zxing.client.android.R.id.zxing_barcode_scanner);
    active[0]=view!=null&&view.getBarcodeView().isPreviewActive();
   });return active[0];},"Camera preview did not start");
   test.runOnMainSync(()->{if(capture.isFinishing()||capture.isDestroyed())throw new IllegalStateException("Scanner exited unexpectedly");capture.finish();});
   test.waitForIdleSync();
   test.await(()->test.activity.status.getText().toString().contains("cancelled"),"Scanner cancellation did not return to fallback");
   Bundle result=new Bundle();result.putString("stage","real-scanner-launch-and-cancel");test.sendStatus(0,result);
  }finally{test.removeMonitor(monitor);}
 }
}
