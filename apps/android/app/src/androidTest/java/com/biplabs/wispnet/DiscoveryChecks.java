package com.biplabs.wispnet;

import android.app.*;
import android.content.*;
import android.os.*;
import android.view.accessibility.AccessibilityNodeInfo;
import java.io.*;
import java.nio.charset.StandardCharsets;
import org.json.*;

/** Actual Binder callers run in the example application's independent UID. */
final class DiscoveryChecks {
 static AccessibilityNodeInfo find(AccessibilityNodeInfo node,String text){
  if(node==null)return null;
  if(node.getText()!=null&&node.getText().toString().equalsIgnoreCase(text))return node;
  if(node.getContentDescription()!=null&&node.getContentDescription().toString().equalsIgnoreCase(text))return node;
  for(int i=0;i<node.getChildCount();i++){AccessibilityNodeInfo found=find(node.getChild(i),text);if(found!=null)return found;}return null;
 }
 static boolean visible(AppTestRunner t,String text){return find(t.getUiAutomation().getRootInActiveWindow(),text)!=null;}
 static void click(AppTestRunner t,String text)throws Exception{
  t.await(()->visible(t,text),"Missing UI: "+text);AccessibilityNodeInfo node=find(t.getUiAutomation().getRootInActiveWindow(),text);
  while(node!=null&&!node.isClickable())node=node.getParent();if(node==null||!node.performAction(AccessibilityNodeInfo.ACTION_CLICK))throw new Exception("Cannot click "+text);
 }
 static void launch(AppTestRunner t,String host,String service,String claim)throws Exception{
  Intent intent=new Intent().setComponent(new ComponentName("app.wispnet.discoveryexample","app.wispnet.discoveryexample.MainActivity"))
   .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK|Intent.FLAG_ACTIVITY_CLEAR_TASK).putExtra("wispnet_package",t.getTargetContext().getPackageName())
   .putExtra("host_id",host).putExtra("service_id",service).putExtra("resolve",true);
  if(claim!=null)intent.putExtra("claim_package",claim);
  t.getTargetContext().startActivity(intent);
 }
 static void barrier(AppTestRunner t,String stage)throws Exception{
  File marker=new File(t.getTargetContext().getFilesDir(),"discovery-continue");marker.delete();t.stage(stage);t.await(marker::exists,"Owner operation not completed: "+stage);marker.delete();
 }
 static void run(AppTestRunner t)throws Exception{
  Context context=t.getTargetContext();t.resume();Native.ensure(context);
  String host=t.args.getString("host_a"),service=t.args.getString("fixture");
  String document=new String(java.nio.file.Files.readAllBytes(new File(context.getFilesDir(),"discovery-invitation.json").toPath()),StandardCharsets.UTF_8);
  Native.request(new JSONObject().put("op","pair").put("host",host).put("name","Discovery test phone").put("invitation",document));
  new HostStore(context).paired(host,"Discovery test host");barrier(t,"discovery-pending-owner");
  JSONObject catalog=Native.request(new JSONObject().put("op","catalog").put("host",host));new HostStore(context).catalog(host,catalog);
  t.runOnMainSync(()->context.startForegroundService(new Intent(context,AccessService.class)));
  t.await(()->AccessService.enabled,"Access not enabled");
  launch(t,host,service,"com.android.settings");t.await(()->visible(t,"caller_rejected"),"Caller package spoof accepted");
  launch(t,"a".repeat(64),service,null);t.await(()->visible(t,"host_not_paired"),"Unpaired host accepted");
  launch(t,"bad",service,null);t.await(()->visible(t,"invalid_request"),"Malformed host accepted");
  launch(t,host,"missing-service",null);t.await(()->visible(t,"service_unavailable"),"Unknown service accepted");
  launch(t,host,service+"-tcp",null);t.await(()->visible(t,"unsupported_protocol"),"TCP service exposed as HTTP");
  launch(t,host,service,null);click(t,"Deny");t.await(()->visible(t,"denied"),"Deny not returned");
  if(new AppGrants(context).all().length()!=0)throw new Exception("Denial persisted a grant");
  click(t,"Resolve service");click(t,"Allow");t.await(()->visible(t,"HTTP/1.1 200 OK"),"Independent app did not reach backend");t.stage("discovery-external-http");
  // A service gear lists only that service's grants, including while cached/offline.
  JSONObject grants=new AppGrants(context).all();String grant=grants.keys().next();JSONObject g=grants.getJSONObject(grant);
  JSONObject unrelated=new JSONObject(g.toString()).put("service_id",service+"-tcp").put("label","Other service app");
  JSONObject otherHost=new JSONObject(g.toString()).put("host_id","a".repeat(64)).put("label","Other host app");
  new AppGrants(context).approve("c".repeat(64),unrelated);new AppGrants(context).approve("d".repeat(64),otherHost);
  t.resume();t.await(()->t.activity.spaceShown&&!t.activity.jumping,"Service cards not shown");
  String gearTag="Configure Discovery test host / "+service;
  t.await(()->t.activity.root.findViewWithTag(gearTag)!=null,"Missing service gear");
  t.runOnMainSync(()->t.activity.root.findViewWithTag(gearTag).performClick());
  t.await(()->visible(t,"App permissions"),"Service permissions missing");
  if(t.activity.root.findViewWithTag("Service permissions sheet")==null||t.activity.space.getVisibility()!=android.view.View.GONE)throw new Exception("Permissions did not use the in-screen sheet");
  if(!t.getUiAutomation().getRootInActiveWindow().findAccessibilityNodeInfosByText("Other service app").isEmpty()||!t.getUiAutomation().getRootInActiveWindow().findAccessibilityNodeInfosByText("Other host app").isEmpty())throw new Exception("Service permissions leaked another service or host's grants");
  SpaceChecks.snapshot(t,"service-permissions");
  click(t,"Revoke app.wispnet.discoveryexample");click(t,"Revoke");
  t.await(()->!new AppGrants(context).all().has(grant),"Revocation not persisted");
  t.await(()->visible(t,"No apps have access to this service."),"Native revocation did not finish");
  if(new AppGrants(context).all().length()!=2)throw new Exception("Revoking a service changed unrelated grants");
  SpaceChecks.snapshot(t,"service-gears");
  t.stage("discovery-service-gear-revoked");
  context.startActivity(new Intent().setComponent(new ComponentName("app.wispnet.discoveryexample","app.wispnet.discoveryexample.MainActivity"))
   .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK|Intent.FLAG_ACTIVITY_REORDER_TO_FRONT)); // Keep the old credential in the existing activity.
  click(t,"Call saved endpoint");t.await(()->visible(t,"HTTP/1.1 401 Unauthorized"),"Revoked credential remained usable");
  click(t,"Resolve service");click(t,"Allow");t.await(()->visible(t,"HTTP/1.1 200 OK"),"Reapproval failed");
  barrier(t,"discovery-withdraw-service");click(t,"Resolve service");t.await(()->visible(t,"service_unavailable"),"Host grant withdrawal did not deny discovery");
  context.stopService(new Intent(context,AccessService.class));t.stage("discovery-passed");
 }
}
