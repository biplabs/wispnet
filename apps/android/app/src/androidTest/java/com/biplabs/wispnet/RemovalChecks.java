package com.biplabs.wispnet;
import java.io.File;
import java.nio.file.Files;
import org.json.*;

/** Isolated physical checks for both directions; production mode is identity-only. */
final class RemovalChecks {
 static void barrier(AppTestRunner t,String stage)throws Exception{
  File ack=new File(t.getTargetContext().getFilesDir(),"removal-continue");ack.delete();t.stage(stage);
  t.await(()->ack.exists(),"Owner action timeout: "+stage);ack.delete();
 }
 static JSONObject request(String op,String host)throws Exception{return Native.request(new JSONObject().put("op",op).put("host",host));}
 static void pair(AppTestRunner t,String host,String file,String stage)throws Exception{
  String document=new String(Files.readAllBytes(new File(t.getTargetContext().getFilesDir(),file).toPath()),java.nio.charset.StandardCharsets.UTF_8);
  Native.request(new JSONObject().put("op","pair").put("host",host).put("invitation",document).put("name","Removal test phone"));
  HostStore store=new HostStore(t.getTargetContext());store.paired(host,"Removal fixture");
  try{request("catalog",host);throw new Exception("Pending request unexpectedly approved");}
  catch(Native.Failure denied){store.failed(host,denied);if(!store.all().has(host))throw new Exception("Pending approval erased");}
  barrier(t,stage);JSONObject catalog=request("catalog",host);store.catalog(host,catalog);
  if(catalog.getJSONArray("services").length()!=0||!store.all().has(host))throw new Exception("Empty catalog erased pairing");
 }
 static void run(AppTestRunner t)throws Exception{
  if(!t.getTargetContext().getPackageName().endsWith(".peercheck"))throw new Exception("Removal tests require the isolated peercheck app");
  t.resume();Native.ensure(t.getTargetContext());HostStore store=new HostStore(t.getTargetContext());
  String a=t.args.getString("host_a"),b=t.args.getString("host_b"),mode=t.args.getString("mode");
  if("removal-cold".equals(mode)){
   if(store.all().has(a)||store.all().has(b)||request("pairing_state",b).getBoolean("paired"))throw new Exception("Removed host returned after restart");
   t.stage("cold-verified");return;
  }
  if("removal-legacy".equals(mode)){
   store.paired(a,"Legacy removal fixture");barrier(t,"legacy-ready");store.catalog(a,request("catalog",a));
   t.runOnMainSync(()->t.activity.removeHost(a));t.await(()->!store.all().has(a),"Phone did not remove legacy host");
   barrier(t,"legacy-phone-removed");return;
  }
  pair(t,a,"removal-a.json","approve-a");
  store.failed(a,new Native.Failure("access denied","host_denied"));if(!store.all().has(a))throw new Exception("Generic denial erased pairing");
  barrier(t,"host-offline");
  try{request("catalog",a);throw new Exception("Offline host unexpectedly answered");}catch(Native.Failure unavailable){store.failed(a,unavailable);}
  if(!store.all().has(a))throw new Exception("Timeout erased pairing");
  barrier(t,"host-online");store.catalog(a,request("catalog",a));
  barrier(t,"delete-a");
  try{request("catalog",a);throw new Exception("Deleted pairing still usable");}
  catch(Native.Failure removed){if(!"pairing_removed".equals(removed.code))throw removed;store.failed(a,removed);}
  if(store.all().has(a)||request("pairing_state",a).getBoolean("paired"))throw new Exception("Remote cleanup affected wrong records or retained consent");
  pair(t,b,"removal-b.json","approve-b");
  store.failed(b,new Native.Failure("Pairing removed by remote machine","pairing_removed"));
  if(!store.all().has(b))throw new Exception("Stale callback erased current pairing");
  t.runOnMainSync(()->t.activity.removeHost(b));t.await(()->!store.all().has(b),"Phone removal did not clear the card");
  if(request("pairing_state",b).getBoolean("paired"))throw new Exception("Phone removal retained native pairing or consent");
  barrier(t,"phone-removed");
 }
 private RemovalChecks(){}
}
