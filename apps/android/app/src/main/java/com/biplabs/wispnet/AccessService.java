package com.biplabs.wispnet;
import android.app.*;
import android.content.*;
import android.net.*;
import android.os.IBinder;
import java.util.concurrent.*;

/** Owns browser access independently of the activity. Never restarts after process death. */
public final class AccessService extends Service {
 public static volatile boolean enabled=false;
 public static volatile String networkStatus="Access stopped";
 private final ExecutorService networkWork=Executors.newSingleThreadExecutor();
 private final Object networkLock=new Object();
 private boolean destroyed=false,worker=false,starting=false;
 private long generation=0;
 private Network current;
 private ConnectivityManager manager;
 private ConnectivityManager.NetworkCallback callback;
 public void onCreate(){
  super.onCreate();NotificationManager nm=getSystemService(NotificationManager.class);
  nm.createNotificationChannel(new NotificationChannel("access","Wispnet browser access",NotificationManager.IMPORTANCE_LOW));
  Intent stop=new Intent(this,AccessService.class).setAction("stop");
  PendingIntent pending=PendingIntent.getService(this,2,stop,PendingIntent.FLAG_IMMUTABLE|PendingIntent.FLAG_UPDATE_CURRENT);
  PendingIntent open=PendingIntent.getActivity(this,1,new Intent(this,MainActivity.class),PendingIntent.FLAG_IMMUTABLE);
  Notification notification=new Notification.Builder(this,"access").setSmallIcon(android.R.drawable.ic_menu_share).setContentTitle("Wispnet access enabled").setContentText("Private browser connections to your paired hosts").setContentIntent(open).addAction(new Notification.Action.Builder(null,"Stop access",pending).build()).setOngoing(true).build();
  startForeground(1,notification);
  manager=getSystemService(ConnectivityManager.class);current=manager.getActiveNetwork();
  networkStatus=current==null?"No network. Reconnect, then reopen the service.":"Network available";
  callback=new ConnectivityManager.NetworkCallback(){
   public void onAvailable(Network network){synchronized(networkLock){if(destroyed||network.equals(current))return;current=network;changed();}}
   public void onLost(Network network){synchronized(networkLock){if(destroyed||!network.equals(current))return;current=null;changed();}}
  };manager.registerDefaultNetworkCallback(callback);
 }
 // Call while holding networkLock. Drain every generation, including a callback
 // arriving during native recovery; coalescing must never lose the final network.
 private void changed(){
  generation++;networkStatus=current==null?"Network lost. Requests interrupted; reconnect and reload the page.":"Network changed. Reload the page; requests are not retried.";
  if(worker)return;worker=true;
  networkWork.execute(()->{while(true){long observed;synchronized(networkLock){if(destroyed){worker=false;return;}observed=generation;}
   try{Native.command("network");}catch(Exception ignored){networkStatus="Network recovery failed. Stop and start access, then reopen the service.";}
   synchronized(networkLock){if(destroyed||observed==generation){worker=false;return;}}
  }});
 }
 public int onStartCommand(Intent intent,int flags,int startId){
  if(intent!=null&&"stop".equals(intent.getAction())){stopSelf();return START_NOT_STICKY;}
  synchronized(networkLock){if(destroyed||starting||enabled)return START_NOT_STICKY;starting=true;}
  networkWork.execute(()->{try{Native.ensure(this);synchronized(networkLock){if(destroyed)return;Native.command("enable");enabled=true;}}catch(Exception failure){networkStatus="Cannot start browser access";stopSelf();}finally{synchronized(networkLock){starting=false;}}});
  return START_NOT_STICKY;
 }
 public void onDestroy(){
  enabled=false;synchronized(networkLock){destroyed=true;}
  try{if(callback!=null)manager.unregisterNetworkCallback(callback);}catch(Exception ignored){}
  try{Native.command("stop");}catch(Exception ignored){}
  networkWork.shutdown();networkStatus="Access stopped. Go online to use your saved service addresses.";
  stopForeground(STOP_FOREGROUND_REMOVE);super.onDestroy();
 }
 @Override public void onTimeout(int startId,int fgsType){stopSelf();}
 public IBinder onBind(Intent intent){return null;}
}
