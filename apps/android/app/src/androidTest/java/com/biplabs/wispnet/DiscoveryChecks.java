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
 static int registryPort=18400;
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
   .putExtra("host_id",host).putExtra("service_id",service).putExtra("registry_port",registryPort).putExtra("resolve",true);
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
  int servicePort;
  try(java.net.ServerSocket registry=new java.net.ServerSocket(0,1,java.net.InetAddress.getLoopbackAddress());java.net.ServerSocket gateway=new java.net.ServerSocket(0,1,java.net.InetAddress.getLoopbackAddress())){registryPort=registry.getLocalPort();servicePort=gateway.getLocalPort();}
  JSONObject settings=Native.command("gateway_settings").getJSONObject("settings");settings.put("registry_port",registryPort).put("service_ports",new JSONArray().put(new JSONObject().put("host",host).put("service",service).put("port",servicePort)));
  Native.request(new JSONObject().put("op","configure_access").put("settings",settings));
  t.runOnMainSync(()->context.startForegroundService(new Intent(context,AccessService.class)));
  t.await(()->AccessService.enabled,"Access not enabled");
  JSONArray routes=Native.command("gateway_settings").getJSONArray("routes");boolean bound=false;for(int i=0;i<routes.length();i++){JSONObject r=routes.getJSONObject(i);if(host.equals(r.getString("host"))&&service.equals(r.getString("service"))&&r.getInt("port")==servicePort)bound=true;}if(!bound)throw new Exception("Fixed service port was not opened on startup");
  launch(t,host,service,null);t.await(()->visible(t,"HTTP/1.1 200 OK"),"Independent HTTP registry client did not reach backend");t.stage("discovery-external-http");
  t.resume();t.await(()->t.activity.spaceShown&&!t.activity.jumping,"Service cards not shown");
  String gearTag="Configure Discovery test host / "+service;
  t.await(()->t.activity.root.findViewWithTag(gearTag)!=null,"Missing service gear");
  t.runOnMainSync(()->t.activity.root.findViewWithTag(gearTag).performClick());
  t.await(()->visible(t,"Service gateway port"),"Service port setting missing");
  if(t.activity.root.findViewWithTag("Service settings sheet")==null)throw new Exception("Settings did not use the in-screen sheet");
  SpaceChecks.snapshot(t,"service-settings");
  barrier(t,"discovery-withdraw-service");launch(t,host,service,null);t.await(()->visible(t,"service_unavailable"),"Host grant withdrawal did not deny discovery");
  context.stopService(new Intent(context,AccessService.class));t.stage("discovery-passed");
 }
}
