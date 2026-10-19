package com.biplabs.wispnet;
import java.io.*;
import java.net.*;
import java.nio.file.Files;
import java.util.concurrent.*;
import org.json.*;

/** Runs only in the separate peercheck application; never clears the user's app. */
final class PeerChecks {
 static void barrier(AppTestRunner t,String name)throws Exception{File ack=new File(t.getTargetContext().getFilesDir(),"peers-continue");ack.delete();t.stage(name);t.await(()->ack.exists(),"Owner action timeout: "+name);ack.delete();}
 static void refresh(AppTestRunner t){t.runOnMainSync(()->t.activity.refresh());}
 static void available(AppTestRunner t,String host)throws Exception{long prior=new HostStore(t.getTargetContext()).all().getJSONObject(host).optLong("fetched");t.await(()->{refresh(t);JSONObject record=new HostStore(t.getTargetContext()).all().getJSONObject(host);return "available".equals(record.optString("state"))&&record.optLong("fetched")>prior;},"Fresh host catalog unavailable");}
 static void browser(AppTestRunner t,String name)throws Exception{t.resume();t.click("Open "+name+" / "+t.args.getString("fixture"));t.await(()->t.browserText("Peer fixture: "+name),"Wrong host or browser fixture");}
 static void start(AppTestRunner t)throws Exception{if(!AccessService.enabled)t.click("Start browser access");t.await(()->AccessService.enabled,"Access not enabled");}
 static void denied(AppTestRunner t,String host)throws Exception{t.await(()->{try{Native.request(new JSONObject().put("op","catalog").put("host",host));throw new AssertionError("Revoked approval was resurrected");}catch(Native.Failure failure){return "host_denied".equals(failure.code);}},"Restarted host did not report durable denial within 45 seconds");}
 static void run(AppTestRunner t)throws Exception{
  t.resume();Native.ensure(t.getTargetContext());String target=t.args.getString("host_a"),other=t.args.getString("host_b"),name=t.args.getString("name"),mode=t.args.getString("mode");
  if(mode.equals("peers-pair")){
   HostStore store=new HostStore(t.getTargetContext());boolean hadOther=store.all().has(other);
   String document=new String(Files.readAllBytes(new File(t.getTargetContext().getFilesDir(),"peer-invitation.json").toPath()),java.nio.charset.StandardCharsets.UTF_8);
   try{Native.request(new JSONObject().put("op","pair").put("host",other).put("invitation",document));throw new Exception("Wrong host pin accepted");}catch(Native.Failure expected){}
   if(!hadOther&&store.all().has(other))throw new Exception("Wrong pin saved");
   String code="wispnet1:"+android.util.Base64.encodeToString(document.getBytes(java.nio.charset.StandardCharsets.UTF_8),android.util.Base64.URL_SAFE|android.util.Base64.NO_PADDING|android.util.Base64.NO_WRAP);
   if(name.equals("Local computer")){
    try{PairingQr.decode(new ByteArrayInputStream(new byte[4*1024*1024+1]));throw new Exception("Oversized QR accepted");}catch(IOException expected){}
    try{PairingQr.decode(new ByteArrayInputStream(new byte[]{1,2,3}));throw new Exception("Malformed QR accepted");}catch(IOException expected){}
    t.runOnMainSync(()->t.activity.onActivityResult(2,android.app.Activity.RESULT_CANCELED,null));
    if(!t.activity.status.getText().toString().contains("paste a pairing code")||t.find(t.activity.root,"Import QR image")==null||t.find(t.activity.root,"Import invitation file")!=null)throw new Exception("Camera cancellation lost fallback");
    String decoded;try(InputStream input=new FileInputStream(new File(t.getTargetContext().getFilesDir(),"peer-invitation.png"))){decoded=PairingQr.decode(input);}
    t.runOnMainSync(()->t.activity.reviewInvitation(decoded));t.await(()->t.activity.status.getText().toString().contains("Pairing code ready"),"QR image review failed");
    if(!target.equals(t.activity.host.getText().toString())||store.all().has(target))throw new Exception("QR image implied verified pin or trust");
    t.runOnMainSync(()->t.activity.onActivityResult(2,android.app.Activity.RESULT_OK,new android.content.Intent().putExtra("SCAN_RESULT","https://untrusted.invalid/invitation")));
    t.await(()->t.activity.status.getText().toString().contains("Cannot read pairing code"),"QR URL was accepted");
    if(store.all().has(target)||!decoded.equals(t.activity.invitation.getText().toString()))throw new Exception("Invalid QR erased the entered code or trusted a host");
    new File(t.getTargetContext().getFilesDir(),"peer-invitation.png").delete();

    android.content.Intent share=new android.content.Intent(android.content.Intent.ACTION_SEND).setClass(t.getTargetContext(),MainActivity.class).setType("text/plain").putExtra(android.content.Intent.EXTRA_TEXT,code).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK|android.content.Intent.FLAG_ACTIVITY_CLEAR_TOP);
    t.activity=(MainActivity)t.startActivitySync(share);
   }else t.runOnMainSync(()->t.activity.reviewInvitation(code));
   t.await(()->t.activity.status.getText().toString().contains("Pairing code ready"),"Camera-free import failed");
   if(!target.equals(t.activity.host.getText().toString())||store.all().has(target))throw new Exception("Import implied verified pin or pairing");
   // Editing a reviewed code invalidates the preview and cannot submit stale trust.
   t.runOnMainSync(()->{t.activity.invitation.setText("wispnet1:invalid");t.activity.pair();});
   if(store.all().has(target)||!t.activity.status.getText().toString().contains("Review the pairing code first"))throw new Exception("Edited code reused stale target");
   t.runOnMainSync(()->t.activity.reviewInvitation(code));
   t.await(()->t.activity.status.getText().toString().contains("Pairing code ready"),"Pairing code review failed");
   t.runOnMainSync(()->t.activity.label.setText(name));
   if(name.equals("Local computer"))t.runOnMainSync(()->t.activity.onActivityResult(2,android.app.Activity.RESULT_OK,new android.content.Intent().putExtra("SCAN_RESULT",code)));
   else t.click("Request pairing");
   t.await(()->store.all().has(target)&&"approval_pending".equals(store.all().getJSONObject(target).optString("state")),"Pairing did not remain pending");
   refresh(t);Thread.sleep(1500);if(!"approval_pending".equals(store.all().getJSONObject(target).optString("state")))throw new Exception("Pending pairing falsely marked approved");
   barrier(t,"pending-owner-approval");available(t,target);
   JSONObject original=store.all().getJSONObject(target).getJSONObject("catalog");
   JSONObject wrong=new JSONObject(original.toString()).put("host",other);
   try{store.catalog(target,wrong);throw new Exception("Foreign catalog owner accepted");}catch(org.json.JSONException invalid){throw invalid;}catch(Exception expected){if("Foreign catalog owner accepted".equals(expected.getMessage()))throw expected;}
   JSONObject stale=new JSONObject(original.toString()).put("revision",original.getLong("revision")-1);
   try{store.catalog(target,stale);throw new Exception("Catalog rollback accepted");}catch(Exception expected){if("Catalog rollback accepted".equals(expected.getMessage()))throw expected;}
   if(!original.toString().equals(store.all().getJSONObject(target).getJSONObject("catalog").toString()))throw new Exception("Rejected catalog altered cache");
   try{Native.request(new JSONObject().put("op","pair").put("host",target).put("invitation",document));throw new Exception("Approved invitation replay accepted");}catch(Native.Failure expected){}
   new File(t.getTargetContext().getFilesDir(),"peer-invitation.json").delete();start(t);browser(t,name);t.stage("paired-and-browser-with-other-host-offline");
  }else if(mode.equals("peers-cold")){
   available(t,target);start(t);browser(t,name);t.stage("cold-browser-with-other-host-offline");
  }else if(mode.equals("peers-service-deny")){
   available(t,target);available(t,other);start(t);LifecycleChecks.Session a=new LifecycleChecks.Session(t,target),b=new LifecycleChecks.Session(t,other);
   Socket stream=a.socket("/stream","GET");stream.setSoTimeout(15000);BufferedReader lines=new BufferedReader(new InputStreamReader(stream.getInputStream()));String line;while((line=lines.readLine())!=null&&!line.startsWith("data:")){}
   if(line==null)throw new Exception("Grant test stream did not start");ExecutorService reader=Executors.newSingleThreadExecutor();try{
    Future<Boolean> ended=reader.submit(()->{try{while(lines.readLine()!=null){}return true;}catch(IOException denied){return true;}finally{stream.close();}});
    barrier(t,"withdraw-service-grant");ended.get(8,TimeUnit.SECONDS);
    if(!a.request("/").startsWith("HTTP/1.1 403")||!b.request("/").contains("Peer fixture: Desktop"))throw new Exception("Service grant denial/isolation failed");
    try{Native.request(new JSONObject().put("op","open").put("host",target).put("service",t.args.getString("fixture")).put("incarnation","a".repeat(64)));throw new Exception("Withdrawn service opened");}catch(Native.Failure denied){if(!"service_denied".equals(denied.code))throw denied;}
    available(t,target);JSONObject record=new HostStore(t.getTargetContext()).all().getJSONObject(target);
    if(record.getJSONObject("catalog").getJSONArray("services").length()!=0)throw new Exception("Withdrawn service remained in catalog");
    barrier(t,"restore-service-grant");available(t,target);browser(t,name);t.stage("service-withdrawal-stream-and-catalog-isolation");
   }finally{reader.shutdownNow();}
  }else if(mode.equals("peers-revoke")){
   available(t,target);available(t,other);start(t);
   LifecycleChecks.Session a=new LifecycleChecks.Session(t,target),b=new LifecycleChecks.Session(t,other);
   Socket stream=a.socket("/stream","GET");stream.setSoTimeout(15000);BufferedReader lines=new BufferedReader(new InputStreamReader(stream.getInputStream()));String line;while((line=lines.readLine())!=null&&!line.startsWith("data:")){}
   if(line==null)throw new Exception("Active stream never started");ExecutorService reader=Executors.newSingleThreadExecutor();try{
    Future<Boolean> ended=reader.submit(()->{try{while(lines.readLine()!=null){}return true;}catch(IOException revoked){return true;}finally{stream.close();}});
    barrier(t,"revoke-active-host");ended.get(8,TimeUnit.SECONDS);
    if(!b.request("/").contains("Peer fixture: Desktop"))throw new Exception("Revocation affected unrelated host session");
    String old=a.request("/");if(!old.startsWith("HTTP/1.1 403"))throw new Exception("Old browser session bypassed host revoke");
    // Click the cached entry before refreshing: a cached listing cannot authorize Open.
    String incarnation=new HostStore(t.getTargetContext()).all().getJSONObject(target).getJSONObject("catalog").getJSONArray("services").getJSONObject(0).getString("incarnation");
    t.runOnMainSync(()->t.activity.open(target,t.args.getString("fixture"),incarnation));
    t.await(()->"host_denied".equals(new HostStore(t.getTargetContext()).all().getJSONObject(target).optString("state")),"UI did not distinguish host denial");
    t.runOnMainSync(()->{android.widget.Button button=t.find(t.activity.root,"Open Local computer / "+t.args.getString("fixture"));if(button==null||button.isEnabled())throw new AssertionError("Denied cached entry still enabled");});
    barrier(t,"restart-revoked-host");
    denied(t,target);
    if(!new HostStore(t.getTargetContext()).all().getJSONObject(target).has("catalog"))throw new Exception("Denial discarded cache provenance");
    t.stage("revocation-stream-cache-isolation-and-restart");
   }finally{reader.shutdownNow();}
  }else if(mode.equals("peers-denied-cold")){
   denied(t,target);
   available(t,other);start(t);browser(t,"Desktop");t.stage("cold-revocation-isolated");
  }else throw new Exception("Unknown peer check mode");
 }
}
