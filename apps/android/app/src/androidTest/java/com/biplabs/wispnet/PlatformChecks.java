package com.biplabs.wispnet;
import android.content.Intent;
import android.os.Build;

/** Uses Android 15's actual framework timeout, not a direct onTimeout call. */
final class PlatformChecks {
 static void run(AppTestRunner t)throws Exception{
  if(Build.VERSION.SDK_INT<35)throw new Exception("Platform timeout test requires Android 15+");
  t.resume();Native.ensure(t.getTargetContext());String peer=Native.command("identity").getString("peer");
  if(new HostStore(t.getTargetContext()).all().length()==0){new HostStore(t.getTargetContext()).paired(peer,"Framework test host");t.resume();}
  t.click("Start browser access");t.await(()->AccessService.enabled&&Native.command("access_status").getBoolean("enabled"),"Access did not start");t.stage("platform-access-started");
  t.getTargetContext().startActivity(new Intent(android.provider.Settings.ACTION_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
  t.await(()->!AccessService.enabled,"Android 15 did not stop the timed-out foreground service");
  if(Native.command("access_status").getBoolean("enabled")||!peer.equals(Native.command("identity").getString("peer")))throw new Exception("Timeout left native access enabled or replaced identity");
  t.stage("framework-timeout-stopped-native-access");
  t.resume();t.click("Start browser access");t.await(()->AccessService.enabled,"Explicit foreground restart after timeout failed");t.await(()->t.find(t.activity.root,"Stop browser access")!=null,"Restart control did not reflect active access");t.click("Stop browser access");t.await(()->!AccessService.enabled,"Stop after restart failed");
  t.stage("platform-explicit-restart-and-stop");
 }
}
