package com.biplabs.wispnet;
import java.io.*;
import java.net.*;
import java.nio.file.Files;
import org.json.*;

/** Remaining device acceptance; secrets stay inside the test app's sandbox. */
final class AcceptanceChecks {
 static void pairUi(AppTestRunner t,String target,String name,String document)throws Exception{
  t.resume();t.runOnMainSync(()->t.activity.reviewInvitation("wispnet1:"+android.util.Base64.encodeToString(document.getBytes(java.nio.charset.StandardCharsets.UTF_8),android.util.Base64.URL_SAFE|android.util.Base64.NO_PADDING|android.util.Base64.NO_WRAP)));t.await(()->t.activity.status.getText().toString().contains("Pairing code ready"),"Pairing code review failed");t.runOnMainSync(()->t.activity.label.setText(name));t.click("Request pairing");
 }
 static void run(AppTestRunner t)throws Exception{
  t.resume();Native.ensure(t.getTargetContext());String target=t.args.getString("host_a"),other=t.args.getString("host_b"),name=t.args.getString("name"),mode=t.args.getString("mode");
  if(mode.equals("peers-egress")){
   javax.net.ssl.HttpsURLConnection connection=(javax.net.ssl.HttpsURLConnection)new URL("https://api.ipify.org").openConnection();connection.setConnectTimeout(15000);connection.setReadTimeout(15000);connection.setInstanceFollowRedirects(false);
   try{String ip=new String(connection.getInputStream().readNBytes(65),java.nio.charset.StandardCharsets.US_ASCII).trim();if(!ip.matches("[0-9]{1,3}(\\.[0-9]{1,3}){3}")||ip.equals(t.args.getString("home_ip")))throw new Exception("External uplink not independently demonstrated");InetAddress address=InetAddress.getByName(ip);if(address.isSiteLocalAddress()||address.isLoopbackAddress()||address.isLinkLocalAddress()||address.isAnyLocalAddress())throw new Exception("Invalid egress address");t.stage("external-public-egress-differs-from-home");}finally{connection.disconnect();}
  }else if(mode.equals("peers-repair")){
   String peer=Native.command("identity").getString("peer");HostStore store=new HostStore(t.getTargetContext());JSONObject original=store.all().getJSONObject(target);PeerChecks.denied(t,target);
   String document=new String(Files.readAllBytes(new File(t.getTargetContext().getFilesDir(),"peer-invitation.json").toPath()),java.nio.charset.StandardCharsets.UTF_8);
   String expired=new JSONObject(document).put("expires_at",System.currentTimeMillis()/1000-1).toString();
   pairUi(t,target,name,expired);t.await(()->t.activity.status.getText().toString().contains("Expired invitation"),"Expired invitation UI unclear");
   if(!"host_denied".equals(store.all().getJSONObject(target).optString("state"))||!original.getJSONObject("catalog").toString().equals(store.all().getJSONObject(target).getJSONObject("catalog").toString()))throw new Exception("Expired invitation reset denied trust state");
   PeerChecks.barrier(t,"reject-repair-invitation");pairUi(t,target,name,document);
   t.await(()->t.activity.status.getText().toString().contains("Ask its owner for a new invitation"),"Rejected invitation UI unclear");
   if(!"host_denied".equals(store.all().getJSONObject(target).optString("state")))throw new Exception("Rejected invitation restored access");
   PeerChecks.barrier(t,"fresh-repair-invitation");document=new String(Files.readAllBytes(new File(t.getTargetContext().getFilesDir(),"peer-invitation.json").toPath()),java.nio.charset.StandardCharsets.UTF_8);
   pairUi(t,target,name,document);t.await(()->"approval_pending".equals(store.all().getJSONObject(target).optString("state")),"Repair skipped independent approval");
   PeerChecks.denied(t,target);PeerChecks.barrier(t,"approve-repair-invitation");PeerChecks.available(t,target);PeerChecks.available(t,other);
   if(!peer.equals(Native.command("identity").getString("peer")))throw new Exception("Re-pair rotated identity");PeerChecks.start(t);PeerChecks.browser(t,name);t.stage("expired-rejected-and-explicit-repair-preserved-identity");
   new File(t.getTargetContext().getFilesDir(),"peer-invitation.json").delete();
  }else if(mode.equals("peers-expiry")){
   PeerChecks.available(t,target);PeerChecks.available(t,other);PeerChecks.start(t);PeerChecks.browser(t,name);
   LifecycleChecks.Session session=new LifecycleChecks.Session(t,target);String peer=Native.command("identity").getString("peer");long began=android.os.SystemClock.elapsedRealtime();int events=0;
   try(Socket stream=session.socket("/stream","GET")){BufferedReader reader=new BufferedReader(new InputStreamReader(stream.getInputStream()));String line;t.stage("ten-minute-stream-started");while((line=reader.readLine())!=null){if(line.startsWith("data:"))events++;if(android.os.SystemClock.elapsedRealtime()-began>=610000)break;}}
   long elapsed=android.os.SystemClock.elapsedRealtime()-began;
   if(elapsed<600000||elapsed>630000||events<2000||!AccessService.enabled)throw new Exception("Persistent session stream failed: elapsed_ms="+elapsed+", events="+events+", enabled="+AccessService.enabled);
   if(!session.request("/").startsWith("HTTP/1.1 200"))throw new Exception("Bookmark expired after ten minutes");t.resume();
   PeerChecks.available(t,target);PeerChecks.available(t,other);PeerChecks.browser(t,name);if(!peer.equals(Native.command("identity").getString("peer")))throw new Exception("Session expiry changed pairing");t.stage("persistent-stream-survived-ten-minutes-without-pairing");
  }else throw new Exception("Unknown acceptance mode");
 }
}
