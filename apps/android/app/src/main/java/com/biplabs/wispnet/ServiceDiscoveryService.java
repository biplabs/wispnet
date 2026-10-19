package com.biplabs.wispnet;

import android.app.*;
import android.content.*;
import android.net.Uri;
import android.os.*;
import java.util.concurrent.*;
import org.json.*;

/** Versioned Messenger protocol: only resolve; never forwards native commands. */
public final class ServiceDiscoveryService extends Service {
 public static final int RESOLVE=1;
 private final ThreadPoolExecutor work=new ThreadPoolExecutor(2,2,0,TimeUnit.SECONDS,new ArrayBlockingQueue<>(16));
 private Messenger incoming;
 public void onCreate(){super.onCreate();incoming=new Messenger(new Handler(getMainLooper()){
  public void handleMessage(Message message){
   int uid=message.sendingUid;Messenger reply=message.replyTo;
   if(reply==null)return;
   try{
    Bundle data=message.getData();String request=data.getString("request_id","");
    if(request.length()>128)throw new IllegalArgumentException();
    if(message.what!=RESOLVE||data.getInt("api_version",0)!=1){send(reply,request,"unsupported_version",null);return;}
    String pkg=data.getString("package_name",""),host=data.getString("host_id",""),service=data.getString("service_id","");
    if(pkg.length()>255||!host.matches("[0-9a-f]{64}")||!service.matches("[a-z0-9][a-z0-9._-]{0,63}")){send(reply,request,"invalid_request",null);return;}
    try{work.execute(()->resolve(uid,pkg,host,service,request,reply));}
    catch(RejectedExecutionException busy){send(reply,request,"busy",null);}
   }catch(Exception invalid){send(reply,"","invalid_request",null);}
  }
 });}
 private void resolve(int uid,String pkg,String host,String service,String request,Messenger reply){
  try{
   String identity;
   try{identity=AppGrants.identity(this,uid,pkg);}catch(Exception invalid){throw new SecurityException("Caller identity unavailable");}
   HostStore store=new HostStore(this);JSONObject record=store.all().optJSONObject(host);
   if(record==null){send(reply,request,"host_not_paired",null);return;}
   Native.ensure(this);
   JSONObject catalog=Native.request(new JSONObject().put("op","catalog").put("host",host));
   store.catalog(host,catalog);
   JSONObject selected=null;JSONArray services=catalog.getJSONArray("services");
   for(int i=0;i<services.length();i++)if(service.equals(services.getJSONObject(i).getString("id")))selected=services.getJSONObject(i);
   if(selected==null){send(reply,request,"service_unavailable",null);return;}
   if(!"http".equals(selected.getString("kind"))){send(reply,request,"unsupported_protocol",null);return;}
   String incarnation=selected.getString("incarnation"),grant=AppGrants.key(identity,host,service,incarnation);
   synchronized(AppGrants.LOCK){
    if(!new AppGrants(this).all().has(grant)){
     Intent consent=new Intent(this,ServiceConsentActivity.class).setData(Uri.parse("wispnet-consent:"+grant))
      .putExtra("uid",uid).putExtra("package_name",pkg).putExtra("identity",identity).putExtra("host_id",host)
      .putExtra("service_id",service).putExtra("incarnation",incarnation).putExtra("host_name",record.optString("name","System"));
     Bundle result=new Bundle();result.putParcelable("consent",PendingIntent.getActivity(this,0,consent,PendingIntent.FLAG_IMMUTABLE|PendingIntent.FLAG_UPDATE_CURRENT));
     send(reply,request,"permission_required",result);return;
    }
    if(!AccessService.enabled){send(reply,request,"offline",null);return;}
    // Recheck identity under the grant lock, and mint only after a fresh catalog.
    if(!identity.equals(AppGrants.identity(this,uid,pkg)))throw new SecurityException();
    JSONObject opened=Native.request(new JSONObject().put("op","app_open").put("host",host).put("service",service)
     .put("incarnation",incarnation).put("host_name",record.optString("name","System")).put("grant",grant));
    Bundle result=new Bundle();result.putString("url",opened.getString("url"));result.putString("header_name","X-Wispnet-Access");
    result.putString("access_token",opened.getString("access_token"));result.putInt("expires_in_seconds",opened.getInt("expires_in_seconds"));
    result.putString("host_id",host);result.putString("service_id",service);
    send(reply,request,"ok",result);
   }
  }catch(SecurityException denied){send(reply,request,"caller_rejected",null);}
  catch(Native.Failure failure){try{new HostStore(this).failed(host,failure);}catch(Exception ignored){}String status="pairing_removed".equals(failure.code)?"host_not_paired":"host_denied".equals(failure.code)?"host_denied":"service_unavailable";send(reply,request,status,null);}
  catch(Exception failure){send(reply,request,"service_unavailable",null);}
 }
 static void send(Messenger reply,String request,String status,Bundle result){
  if(result==null)result=new Bundle();result.putInt("api_version",1);result.putString("request_id",request);result.putString("status",status);
  Message response=Message.obtain(null,RESOLVE);response.setData(result);try{reply.send(response);}catch(RemoteException ignored){}
 }
 public IBinder onBind(Intent intent){return incoming.getBinder();}
 public void onDestroy(){work.shutdownNow();super.onDestroy();}
}
