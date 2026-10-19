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
 static void buttonLayout(AppTestRunner t)throws Exception {
  final Bitmap[] bitmap={null};
  t.runOnMainSync(()->{
   SpaceToggle button=new SpaceToggle(t.getTargetContext());button.setSelected(true);
   float density=button.getResources().getDisplayMetrics().density;
   int width=button.dockedWidth(),height=(int)Math.ceil(44*density);
   button.measure(View.MeasureSpec.makeMeasureSpec(width,View.MeasureSpec.EXACTLY),View.MeasureSpec.makeMeasureSpec(height,View.MeasureSpec.EXACTLY));
   button.layout(0,0,width,height);
   bitmap[0]=Bitmap.createBitmap(width,height,Bitmap.Config.ARGB_8888);button.draw(new Canvas(bitmap[0]));
  });
  try(FileOutputStream out=new FileOutputStream(new File(t.getTargetContext().getFilesDir(),"button-layout.png"))){bitmap[0].compress(Bitmap.CompressFormat.PNG,100,out);}bitmap[0].recycle();
 }
 static void run(AppTestRunner t)throws Exception {
  t.resume();Native.ensure(t.getTargetContext());
  if(t.activity.pairingScroll.getVisibility()!=View.GONE||t.activity.header.getChildAt(0).getVisibility()!=View.GONE||!((SpaceToggle)t.activity.access).plus)throw new Exception("Unpaired home must hide management and hosts pill and show plus");
  t.runOnMainSync(()->t.activity.access.performClick());t.waitForIdleSync();
  if(t.activity.pairingScroll.getVisibility()!=View.VISIBLE)throw new Exception("Pairing sheet did not open from plus");
  t.runOnMainSync(()->t.activity.showPairing(false));t.waitForIdleSync();
  String host="4b5afc25209fc5aac1007be0b5a3f8adf1a1e7b7fdb8663a7f8a4ee2152f0320";
  HostStore store=new HostStore(t.getTargetContext());store.paired(host,"j-strix");store.catalog(host,new JSONObject().put("version",1).put("complete",true).put("service_count",2).put("host",host).put("generation","c".repeat(64)).put("revision",1).put("services",new JSONArray().put(new JSONObject().put("id","llama").put("incarnation","b".repeat(64)).put("display_name","Llama").put("kind","http")).put(new JSONObject().put("id","website").put("incarnation","a".repeat(64)).put("display_name","Website").put("kind","http"))));
  t.runOnMainSync(()->{t.activity.showPairing(false);t.activity.render();});t.waitForIdleSync();
  if(t.activity.servicesScroll.getVisibility()!=View.GONE||t.activity.caption.getVisibility()!=View.VISIBLE||t.activity.status.getVisibility()!=View.GONE||!t.activity.caption.getText().toString().equals("jump to wisp space")||((SpaceToggle)t.activity.access).plus||t.activity.header.getChildAt(0).getVisibility()!=View.VISIBLE||t.activity.header.getChildAt(0).getLeft()>=t.activity.header.getWidth()/2||!t.activity.machinesPill.getText().toString().contains("Manage system"))throw new Exception("Idle home is cluttered or hosts pill is not top-left");
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
