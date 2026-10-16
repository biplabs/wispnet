package com.biplabs.wispnet;
import android.app.*;
import android.content.*;
import android.net.*;
import android.os.IBinder;
import java.util.concurrent.Executors;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.atomic.AtomicBoolean;

public final class AccessService extends Service {
 public static volatile boolean enabled=false;
 private final ExecutorService networkWork=Executors.newSingleThreadExecutor();
 private final AtomicBoolean networkPending=new AtomicBoolean(false);
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
  manager=getSystemService(ConnectivityManager.class);callback=new ConnectivityManager.NetworkCallback(){public void onAvailable(Network network){networkChanged();}public void onLost(Network network){networkChanged();}};manager.registerDefaultNetworkCallback(callback);
 }
 private void networkChanged(){if(!networkPending.compareAndSet(false,true))return;networkWork.execute(()->{try{Native.command("network");}catch(Exception ignored){}finally{networkPending.set(false);}});}
 public int onStartCommand(Intent intent,int flags,int startId){
  if(intent!=null&&"stop".equals(intent.getAction())){stopSelf();return START_NOT_STICKY;}
  try{Native.ensure(this);Native.command("enable");enabled=true;}catch(Exception failure){stopSelf();}
  return START_NOT_STICKY;
 }
 public void onDestroy(){enabled=false;try{if(callback!=null)manager.unregisterNetworkCallback(callback);}catch(Exception ignored){}
  try{Native.command("stop");}catch(Exception ignored){}networkWork.shutdownNow();stopForeground(STOP_FOREGROUND_REMOVE);super.onDestroy();}
 public void onTimeout(int startId,int fgsType){stopSelf();}
 public IBinder onBind(Intent intent){return null;}
}
