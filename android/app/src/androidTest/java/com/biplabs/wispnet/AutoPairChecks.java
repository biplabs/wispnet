package com.biplabs.wispnet;
import android.app.Activity;
import android.content.Intent;
import java.io.File;
import java.nio.file.Files;
/** Exercise the real scanner-result path with no name/checkbox/Request click. */
final class AutoPairChecks {
 static void run(AppTestRunner t)throws Exception {
  t.resume();Native.ensure(t.getTargetContext());
  String target=t.args.getString("host_a");
  t.runOnMainSync(()->t.activity.onActivityResult(2,Activity.RESULT_OK,new Intent().putExtra("SCAN_RESULT","https://untrusted.invalid/")));
  t.await(()->t.activity.status.getText().toString().contains("Cannot import invitation"),"Invalid QR was not rejected");
  if(new HostStore(t.getTargetContext()).all().has(target))throw new Exception("Invalid QR saved trust");
  File source=new File(t.getTargetContext().getFilesDir(),"peer-invitation.json");String document=new String(Files.readAllBytes(source.toPath()),java.nio.charset.StandardCharsets.UTF_8);source.delete();
  t.runOnMainSync(()->{t.activity.label.setText("");t.activity.onActivityResult(2,Activity.RESULT_OK,new Intent().putExtra("SCAN_RESULT",document));});
  // A UI-thread round trip must work while network pairing is in progress.
  t.runOnMainSync(()->{});
  t.await(()->new HostStore(t.getTargetContext()).all().has(target),"QR did not automatically submit pairing");
  t.await(()->t.activity.status.getText().toString().contains("Pending owner approval"),"Missing visible pending feedback");
  if(!"approval_pending".equals(new HostStore(t.getTargetContext()).all().getJSONObject(target).getString("state")))throw new Exception("QR claimed approval");
  if(t.activity.hostVerified.isChecked()||!t.activity.label.getText().toString().equals("Host "+target.substring(0,8)))throw new Exception("QR implied manual verification or omitted default name");
  t.stage("qr-auto-request-pending");
 }
}
