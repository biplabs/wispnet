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
 private final ScheduledExecutorService profileWork=Executors.newSingleThreadScheduledExecutor();
 private final ExecutorService networkWork=Executors.newSingleThreadExecutor();
 private final Object networkLock=new Object();
 private boolean destroyed=false,worker=false,starting=false;
 private long generation=0;
 private boolean interrupted=false;
 private Network current;
 private ConnectivityManager manager;
 private ConnectivityManager.NetworkCallback callback;
 public void onCreate(){
  super.onCreate();profileWork.scheduleWithFixedDelay(()->{if(enabled)DeviceProfile.syncAll(this);},5,30,TimeUnit.SECONDS);NotificationManager nm=getSystemService(NotificationManager.class);
  nm.createNotificationChannel(new NotificationChannel("access","Wispnet browser access",NotificationManager.IMPORTANCE_LOW));
  startForeground(1,notification("Wispnet starting…"));
  manager=getSystemService(ConnectivityManager.class);current=manager.getActiveNetwork();
  networkStatus=current==null?"No network. Reconnect, then reopen the service.":"Network available";
  callback=new ConnectivityManager.NetworkCallback(){
   public void onAvailable(Network network){synchronized(networkLock){if(destroyed||network.equals(current))return;current=network;changed();}}
   public void onLost(Network network){synchronized(networkLock){if(destroyed||!network.equals(current))return;current=null;changed();}}
  };manager.registerDefaultNetworkCallback(callback);
 }
 static String reconnectedMessage(boolean interrupted){return interrupted?"Reconnected. A page was interrupted; reload that page to continue.":"Network available";}
 private Notification notification(String title){
  Intent stop=new Intent(this,AccessService.class).setAction("stop");
  PendingIntent pending=PendingIntent.getService(this,2,stop,PendingIntent.FLAG_IMMUTABLE|PendingIntent.FLAG_UPDATE_CURRENT);
  PendingIntent open=PendingIntent.getActivity(this,1,new Intent(this,MainActivity.class),PendingIntent.FLAG_IMMUTABLE);
  return new Notification.Builder(this,"access").setSmallIcon(R.drawable.ic_stat_wispnet).setContentTitle(title).setContentText("Access to your paired systems is enabled").setContentIntent(open).addAction(new Notification.Action.Builder(null,"Go offline",pending).build()).setOngoing(true).setOnlyAlertOnce(true).setShowWhen(false).setCategory(Notification.CATEGORY_SERVICE).setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE).build();
 }
 // Call while holding networkLock. Drain every generation, including a callback
 // arriving during native recovery; coalescing must never lose the final network.
 private void changed(){
  generation++;networkStatus=current==null?"No network. Waiting for a connection…":"Reconnecting…";
  if(worker)return;worker=true;
  networkWork.execute(()->{while(true){long observed;synchronized(networkLock){if(destroyed){worker=false;return;}observed=generation;}
   boolean failed=false;
   try{
    int count=Native.command("network").optInt("interrupted_requests",0);
    synchronized(networkLock){interrupted|=count>0;}
   }catch(Exception ignored){failed=true;}
   synchronized(networkLock){
    if(destroyed){worker=false;return;}
    if(observed!=generation)continue;
    if(failed)networkStatus="Could not reconnect. Go offline, then online to try again.";
    else if(current!=null){
     networkStatus=reconnectedMessage(interrupted);
     if(interrupted){
      final String notice=networkStatus;
      profileWork.schedule(()->{synchronized(networkLock){if(!destroyed&&observed==generation&&notice.equals(networkStatus))networkStatus="Network available";}},10,TimeUnit.SECONDS);
     }
     interrupted=false;
    }
    worker=false;return;
   }
  }});
 }

 public int onStartCommand(Intent intent,int flags,int startId){
  if(intent!=null&&"stop".equals(intent.getAction())){stopSelf();return START_NOT_STICKY;}
  synchronized(networkLock){if(destroyed||starting||enabled)return START_NOT_STICKY;starting=true;}
  networkWork.execute(()->{try{Native.ensure(this);synchronized(networkLock){if(destroyed)return;Native.command("enable");enabled=true;getSystemService(NotificationManager.class).notify(1,notification("Wispnet online"));}}catch(Exception failure){networkStatus="Cannot start browser access";stopSelf();}finally{synchronized(networkLock){starting=false;}}});
  return START_NOT_STICKY;
 }
 public void onDestroy(){
  enabled=false;synchronized(networkLock){destroyed=true;}
  try{if(callback!=null)manager.unregisterNetworkCallback(callback);}catch(Exception ignored){}
  try{Native.command("stop");}catch(Exception ignored){}
  profileWork.shutdownNow();networkWork.shutdown();networkStatus="Access stopped. Go online to use your saved service addresses.";
  stopForeground(STOP_FOREGROUND_REMOVE);super.onDestroy();
 }
 @Override public void onTimeout(int startId,int fgsType){stopSelf();}
 public IBinder onBind(Intent intent){return null;}
}
