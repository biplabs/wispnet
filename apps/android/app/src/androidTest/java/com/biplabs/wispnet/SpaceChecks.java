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
 static void serviceLaunch(AppTestRunner t)throws Exception {
  t.resume();Native.ensure(t.getTargetContext());
  t.runOnMainSync(()->{t.activity.showPairing(false);if(!AccessService.enabled)t.activity.toggleSpace();});t.await(()->t.activity.spaceShown&&!t.activity.jumping,"Access did not start");
  android.content.IntentFilter filter=new android.content.IntentFilter(android.content.Intent.ACTION_VIEW);
  filter.addDataScheme("http");filter.addCategory(android.content.Intent.CATEGORY_BROWSABLE);
  Instrumentation.ActivityMonitor browser=t.addMonitor(filter,null,true);
  try{
   t.runOnMainSync(()->{
    t.activity.openingService=true;t.activity.pendingServiceUrl="http://launch-test.localhost:1234/";
    t.activity.finishServiceLaunch();
    if(browser.getHits()!=0)throw new AssertionError("Browser launched before animation");
    t.activity.stars.speedUp(()->{t.activity.serviceJumpDone=true;t.activity.finishServiceLaunch();});
   });
   t.runOnMainSync(()->{if(t.activity.root.getAlpha()!=1)throw new AssertionError("Service launch hid the directory");});
   t.await(()->browser.getHits()==1,"Browser did not launch after star speed-up");
   t.runOnMainSync(()->{
    if(t.activity.openingService||t.activity.root.getAlpha()!=1)throw new AssertionError("Launch did not restore UI");
    t.activity.openingService=true;t.activity.serviceJumpDone=true;t.activity.finishServiceLaunch();
    if(browser.getHits()!=1)throw new AssertionError("Browser launched without a URL");
    t.activity.pendingServiceUrl="http://launch-test.localhost:1234/";t.activity.finishServiceLaunch();
    if(browser.getHits()!=2)throw new AssertionError("Slow connection did not launch after completed warp");
    t.activity.openingService=true;t.activity.pendingServiceUrl="http://launch-test.localhost:1234/";
    t.activity.resetServiceLaunch();t.activity.finishServiceLaunch();
    if(browser.getHits()!=2)throw new AssertionError("Cancelled launch opened browser");
   });
  }finally{t.removeMonitor(browser);t.runOnMainSync(()->t.activity.resetServiceLaunch());t.getTargetContext().stopService(new android.content.Intent(t.getTargetContext(),AccessService.class));}
 }
 static void headerLayout(AppTestRunner t)throws Exception {
  t.resume();Native.ensure(t.getTargetContext());
  if(!AccessService.enabled)t.click("Start browser access");
  t.await(()->t.activity.spaceShown&&!t.activity.jumping,"Online screen did not appear");t.waitForIdleSync();
  if(!t.activity.deviceHeading.getText().toString().equals(DeviceProfile.name(t.getTargetContext())+" · "+Native.command("identity").getString("peer").substring(0,8)))throw new Exception("Missing device name");
  if(!t.activity.machinesPill.getText().toString().equals("Manage"))throw new Exception("Wrong paired machines label");
  if(t.activity.access.getParent()!=t.activity.header||t.activity.deviceHeading.getParent().getParent()!=t.activity.header)throw new Exception("Device name and offline button are not in the same header");
  t.await(()->t.activity.relayLabel.getText().toString().startsWith("● Relay: ")&&!t.activity.relayLabel.getText().toString().contains("connecting"),"Missing relay observation");
  snapshot(t,"android-header");
 }
 static void networkFeedback(AppTestRunner t)throws Exception {
  if(!"Network available".equals(AccessService.reconnectedMessage(false)))throw new Exception("Idle reconnect asked to reload");
  if(!AccessService.reconnectedMessage(true).contains("reload"))throw new Exception("Interrupted page missing reload advice");
  t.resume();
  t.runOnMainSync(()->{
   t.activity.updateNetworkFeedback("Reconnecting…",true);
   if(t.activity.status.getVisibility()!=View.VISIBLE)throw new AssertionError("Missing reconnect notice");
   t.activity.updateNetworkFeedback(AccessService.reconnectedMessage(false),true);
   if(t.activity.status.getVisibility()!=View.GONE)throw new AssertionError("Recovered message stayed visible");
   t.activity.updateNetworkFeedback(AccessService.reconnectedMessage(true),true);
   if(!t.activity.status.getText().toString().contains("reload"))throw new AssertionError("Missing interrupted-page notice");
   t.activity.show("Unrelated operation failed");
   t.activity.updateNetworkFeedback("Network available",true);
   if(!t.activity.status.getText().toString().equals("Unrelated operation failed")||t.activity.status.getVisibility()!=View.VISIBLE)throw new AssertionError("Recovery erased unrelated feedback");
   t.activity.hideNotice();
   t.activity.show("First notification");
  });
  Thread.sleep(2500);
  t.runOnMainSync(()->t.activity.show("Updated notification"));
  Thread.sleep(2000);
  t.runOnMainSync(()->{
   if(t.activity.status.getVisibility()!=View.VISIBLE||!t.activity.status.getText().toString().equals("Updated notification"))throw new AssertionError("New notification inherited old dismissal timer");
  });
  t.await(()->t.activity.status.getVisibility()==View.GONE,"Notification did not dismiss automatically");
 }
 static void buttonLayout(AppTestRunner t)throws Exception {
  final Bitmap[] bitmap={null};
  t.runOnMainSync(()->{
   SpaceToggle button=new SpaceToggle(t.getTargetContext());button.setSelected(true);
   float density=button.getResources().getDisplayMetrics().density;
   int width=button.dockedWidth(),height=(int)Math.ceil(44*density);
   button.offlineText(density);android.graphics.Paint.FontMetrics metrics=button.paint.getFontMetrics();
   android.os.Bundle values=new android.os.Bundle();values.putFloat("density",density);values.putFloat("text_width",button.paint.measureText("GO OFFLINE")/density);values.putFloat("text_ascent",metrics.ascent/density);values.putFloat("text_descent",metrics.descent/density);values.putInt("width",width);values.putInt("height",height);
   t.sendStatus(0,values);
   button.measure(View.MeasureSpec.makeMeasureSpec(width,View.MeasureSpec.EXACTLY),View.MeasureSpec.makeMeasureSpec(height,View.MeasureSpec.EXACTLY));
   button.layout(0,0,width,height);
   bitmap[0]=Bitmap.createBitmap(width,height,Bitmap.Config.ARGB_8888);button.draw(new Canvas(bitmap[0]));
  });
  try(FileOutputStream out=new FileOutputStream(new File(t.getTargetContext().getFilesDir(),"button-layout.png"))){bitmap[0].compress(Bitmap.CompressFormat.PNG,100,out);}bitmap[0].recycle();
 }
 static void run(AppTestRunner t)throws Exception {
  t.resume();Native.ensure(t.getTargetContext());
  if(t.activity.pairingScroll.getVisibility()!=View.GONE||t.activity.machinesPill.getVisibility()!=View.GONE||!((SpaceToggle)t.activity.access).plus)throw new Exception("Unpaired home must hide management and hosts pill and show plus");
  t.runOnMainSync(()->t.activity.access.performClick());t.waitForIdleSync();
  if(t.activity.pairingScroll.getVisibility()!=View.VISIBLE)throw new Exception("Pairing sheet did not open from plus");
  t.runOnMainSync(()->t.activity.showPairing(false));t.waitForIdleSync();
  String host="4b5afc25209fc5aac1007be0b5a3f8adf1a1e7b7fdb8663a7f8a4ee2152f0320";
  HostStore store=new HostStore(t.getTargetContext());store.paired(host,"j-strix");store.catalog(host,new JSONObject().put("version",1).put("complete",true).put("service_count",2).put("host",host).put("generation","c".repeat(64)).put("revision",1).put("services",new JSONArray().put(new JSONObject().put("id","llama").put("incarnation","b".repeat(64)).put("display_name","Llama").put("kind","http")).put(new JSONObject().put("id","website").put("incarnation","a".repeat(64)).put("display_name","Website").put("kind","http"))));
  t.runOnMainSync(()->{t.activity.showPairing(false);t.activity.render();});t.waitForIdleSync();
  if(t.activity.servicesScroll.getVisibility()!=View.GONE||t.activity.caption.getVisibility()!=View.VISIBLE||t.activity.status.getVisibility()!=View.GONE||!t.activity.caption.getText().toString().equals("jump to wisp space")||((SpaceToggle)t.activity.access).plus||t.activity.machinesPill.getVisibility()!=View.VISIBLE||!t.activity.machinesPill.getText().toString().contains("Manage"))throw new Exception("Idle home is cluttered or hosts pill is not top-left");
  if(t.activity.stars.getWidth()!=t.activity.screen.getWidth()||t.activity.stars.getHeight()!=t.activity.screen.getHeight())throw new Exception("Warp background is not full bleed");
  snapshot(t,"space-idle");
  t.click("Start browser access");t.await(()->t.activity.jumping||t.activity.spaceShown,"Entry did not start");
  if(android.animation.ValueAnimator.areAnimatorsEnabled()){t.await(()->t.activity.stars.progress>.2f,"Jump animation did not advance");snapshot(t,"space-jump");}
  t.await(()->t.activity.spaceShown&&!t.activity.jumping,"Services not revealed after real access startup");t.waitForIdleSync();
  if(!AccessService.enabled||!t.activity.access.isSelected()||t.activity.servicesScroll.getVisibility()!=View.VISIBLE||t.activity.caption.getVisibility()!=View.GONE||t.activity.access.getParent()!=t.activity.header||t.activity.access.getLeft()<=t.activity.header.getWidth()/2)throw new Exception("Active UI and proxy disagree");
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
