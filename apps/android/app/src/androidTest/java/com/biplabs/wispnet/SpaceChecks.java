package com.biplabs.wispnet;
import android.app.*;
import android.graphics.*;
import android.view.*;
import org.json.*;
import java.io.*;
/** Isolated UI fixtures; no host credentials, approvals or live-service claims. */
final class SpaceChecks {
 static void snapshot(AppTestRunner t,String name)throws Exception {
  final Bitmap[] bitmap={null};t.runOnMainSync(()->{View view=t.activity.screen;bitmap[0]=Bitmap.createBitmap(view.getWidth(),view.getHeight(),Bitmap.Config.ARGB_8888);view.draw(new Canvas(bitmap[0]));});
  try(FileOutputStream out=new FileOutputStream(new File(t.getTargetContext().getFilesDir(),name+".png"))){bitmap[0].compress(Bitmap.CompressFormat.PNG,100,out);}bitmap[0].recycle();
 }
 static void run(AppTestRunner t)throws Exception {
  t.resume();Native.ensure(t.getTargetContext());
  if(t.activity.pairingScroll.getVisibility()!=View.GONE)throw new Exception("Home must hide management");
  android.app.Instrumentation.ActivityMonitor monitor=t.addMonitor(HostManagementActivity.class.getName(),null,false);t.runOnMainSync(()->t.activity.manageHosts());
  MainActivity management=(MainActivity)t.waitForMonitorWithTimeout(monitor,10000);t.removeMonitor(monitor);
  if(management==null)throw new Exception("Management did not open");
  if(!management.managesHosts()||management.pairingScroll.getVisibility()!=View.VISIBLE)throw new Exception("Separate management activity missing");
  t.runOnMainSync(management::finish);t.waitForIdleSync();
  String host="4b5afc25209fc5aac1007be0b5a3f8adf1a1e7b7fdb8663a7f8a4ee2152f0320";
  HostStore store=new HostStore(t.getTargetContext());store.paired(host,"j-strix");store.catalog(host,new JSONObject().put("version",1).put("complete",true).put("service_count",2).put("host",host).put("generation","c".repeat(64)).put("revision",1).put("services",new JSONArray().put(new JSONObject().put("id","llama").put("incarnation","b".repeat(64)).put("display_name","Llama").put("kind","http")).put(new JSONObject().put("id","website").put("incarnation","a".repeat(64)).put("display_name","Website").put("kind","http"))));
  t.runOnMainSync(()->{t.activity.showPairing(false);t.activity.render();});t.waitForIdleSync();
  if(t.activity.servicesScroll.getVisibility()!=View.GONE||t.activity.caption.getVisibility()!=View.VISIBLE||t.activity.status.getVisibility()!=View.GONE||!t.activity.caption.getText().toString().equals("enter wisp space"))throw new Exception("Idle home is cluttered");
  if(t.activity.stars.getWidth()!=t.activity.screen.getWidth()||t.activity.stars.getHeight()!=t.activity.screen.getHeight())throw new Exception("Warp background is not full bleed");
  snapshot(t,"space-idle");
  t.click("Start browser access");t.await(()->t.activity.jumping||t.activity.spaceShown,"Entry did not start");
  if(android.animation.ValueAnimator.areAnimatorsEnabled()){t.await(()->t.activity.stars.progress>.2f,"Jump animation did not advance");snapshot(t,"space-jump");}
  t.await(()->t.activity.spaceShown&&!t.activity.jumping,"Services not revealed after real access startup");
  if(!AccessService.enabled||!t.activity.access.isSelected()||t.activity.servicesScroll.getVisibility()!=View.VISIBLE||t.activity.caption.getVisibility()!=View.GONE)throw new Exception("Active UI and proxy disagree");
  // Reinstate deterministic display-only fixtures after the real unapproved catalog refresh.
  store.paired(host,"j-strix");store.catalog(host,new JSONObject().put("version",1).put("complete",true).put("service_count",2).put("host",host).put("generation","c".repeat(64)).put("revision",1).put("services",new JSONArray().put(new JSONObject().put("id","llama").put("incarnation","b".repeat(64)).put("display_name","Llama").put("kind","http")).put(new JSONObject().put("id","website").put("incarnation","a".repeat(64)).put("display_name","Website").put("kind","http"))));
  t.runOnMainSync(()->t.activity.render());Thread.sleep(500);snapshot(t,"space-services");
  t.runOnMainSync(()->t.activity.recreate());Thread.sleep(1000);t.resume();
  if(!t.activity.spaceShown||t.activity.pairingScroll.getVisibility()!=View.GONE)throw new Exception("Recreation lost active home");
  t.click("Stop browser access");t.await(()->!AccessService.enabled&&!t.activity.spaceShown,"Leave did not stop access");
  if(Native.command("access_status").getBoolean("enabled"))throw new Exception("Stop left native access enabled");
  if(t.activity.access.isSelected()||t.activity.servicesScroll.getVisibility()!=View.GONE)throw new Exception("Stop left active UI");
  t.runOnMainSync(()->t.activity.showPairing(true));t.waitForIdleSync();snapshot(t,"space-pairing");
  t.stage("dark-space-entry-animation-recreation-stop");
 }
}
