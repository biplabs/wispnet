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
  t.await(()->t.activity.status.getText().toString().contains("Cannot read pairing code"),"Invalid QR was not rejected");
  if(new HostStore(t.getTargetContext()).all().has(target))throw new Exception("Invalid QR saved trust");
  File source=new File(t.getTargetContext().getFilesDir(),"peer-invitation.json");String document=new String(Files.readAllBytes(source.toPath()),java.nio.charset.StandardCharsets.UTF_8);source.delete();
  String machine=new org.json.JSONObject(document).optString("host_name","Host "+target.substring(0,8));
  String code="wispnet1:"+android.util.Base64.encodeToString(document.getBytes(java.nio.charset.StandardCharsets.UTF_8),android.util.Base64.URL_SAFE|android.util.Base64.NO_PADDING|android.util.Base64.NO_WRAP);
  if("code-pair".equals(t.args.getString("mode"))){
   t.runOnMainSync(()->{
    android.widget.Button pair=t.find(t.activity.root,"Pair");
    if(pair==null||pair.isEnabled())throw new RuntimeException("Empty pairing code enabled Pair");
    t.activity.invitation.setText(" \n ");if(pair.isEnabled())throw new RuntimeException("Blank pairing code enabled Pair");
    t.activity.invitation.setText("wispnet1:test");if(!pair.isEnabled())throw new RuntimeException("Text did not enable Pair");
    t.activity.invitation.setText("");if(pair.isEnabled())throw new RuntimeException("Clearing text did not disable Pair");
    t.activity.reviewInvitation(document);
   });
   if(!t.activity.status.getText().toString().contains("valid Wispnet pairing code"))throw new Exception("Raw JSON accepted by pairing UI");
   if((t.activity.invitation.getInputType()&android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE)==0)throw new Exception("Pairing field is not multiline");
   t.runOnMainSync(()->t.activity.reviewInvitation(code.substring(0,30)+"\n"+code.substring(30)));
   t.await(()->t.activity.status.getText().toString().contains("Pairing code ready"),"Code preview unavailable");
   if(t.activity.invitation.getTransformationMethod() instanceof android.text.method.PasswordTransformationMethod)throw new Exception("Pairing code was masked");
   if(new HostStore(t.getTargetContext()).all().has(target)||t.activity.host.getKeyListener()!=null||t.find(t.activity.root,"Import invitation file")!=null)throw new Exception("Preview trusted target or retained editable ID/file import");
   t.runOnMainSync(()->{t.activity.invitation.setText("wispnet1:invalid");t.activity.pair();});
   if(!t.activity.status.getText().toString().contains("Review the pairing code first"))throw new Exception("Edited code reused target preview");
   t.runOnMainSync(()->t.activity.reviewInvitation(code));
   t.await(()->t.activity.status.getText().toString().contains("Pairing code ready"),"Fresh preview unavailable");
   t.click("Request pairing");
  }else t.runOnMainSync(()->{t.activity.label.setText("");t.activity.onActivityResult(2,Activity.RESULT_OK,new Intent().putExtra("SCAN_RESULT",code));});
  // A UI-thread round trip must work while network pairing is in progress.
  t.runOnMainSync(()->{});
  t.await(()->new HostStore(t.getTargetContext()).all().has(target),"QR did not automatically submit pairing");
  t.await(()->t.activity.status.getText().toString().contains("Pending owner approval"),"Missing visible pending feedback");
  if(!"approval_pending".equals(new HostStore(t.getTargetContext()).all().getJSONObject(target).getString("state")))throw new Exception("QR claimed approval");
  if(!t.activity.label.getText().toString().equals(machine))throw new Exception("QR omitted default name");
  try{
   Native.request(new org.json.JSONObject().put("op","catalog").put("host",target));
   throw new Exception("Pending host exposed a catalog");
  }catch(Native.Failure failure){
   if(!"approval_pending".equals(failure.code))throw new Exception("Pending request was reported as "+failure.code);
   new HostStore(t.getTargetContext()).failed(target,failure);
  }
  if(!"approval_pending".equals(new HostStore(t.getTargetContext()).all().getJSONObject(target).getString("state")))throw new Exception("Pending refresh became denied");
  t.stage("qr-auto-request-pending");
 }
}
