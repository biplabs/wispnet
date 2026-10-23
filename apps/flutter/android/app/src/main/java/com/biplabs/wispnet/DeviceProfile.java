package com.biplabs.wispnet;
import android.content.*;
import android.os.Build;
import android.provider.Settings;
import org.json.*;

/** One readable device ID, independent of the protected host identity. */
final class DeviceProfile {
 static boolean valid(String id){return id!=null&&id.matches("[a-z0-9][a-z0-9._-]{0,63}");}
 static String readableId(String value){
  if(valid(value))return value;
  StringBuilder id=new StringBuilder();
  for(int cp:value.toLowerCase(java.util.Locale.ROOT).codePoints().toArray()){
   if((cp>='a'&&cp<='z')||(cp>='0'&&cp<='9')||(id.length()>0&&(cp=='.'||cp=='_'||cp=='-')))id.appendCodePoint(cp);
   else if(id.length()>0&&id.charAt(id.length()-1)!='-')id.append('-');
   if(id.length()==64)break;
  }
  while(id.length()>0&&"._-".indexOf(id.charAt(id.length()-1))>=0)id.setLength(id.length()-1);
  return id.length()==0?"device":id.toString();
 }
 // Native initialization reads this while holding its own monitor. Do not take
 // the syncHost monitor here: syncHost calls Native.ensure in the other order.
 static String name(Context context){
  SharedPreferences prefs=context.getSharedPreferences("device_profile",Context.MODE_PRIVATE);
  String saved=prefs.getString("device_id",null);if(valid(saved))return saved;
  String source=prefs.getString("name",null);
  if(source==null||source.trim().isEmpty())try{source=Settings.Global.getString(context.getContentResolver(),"device_name");}catch(SecurityException ignored){}
  if(source==null||source.trim().isEmpty())source=Build.MODEL;
  String id=readableId(source==null?"android-device":source);
  if(!prefs.edit().putString("device_id",id).remove("name").commit())throw new IllegalStateException("Cannot save device ID");
  return id;
 }
 static void save(Context context,String id)throws Exception{
  if(!valid(id))throw new Exception("Use 1–64 lowercase letters, numbers, dots, underscores or hyphens; start with a letter or number");
  if(!context.getSharedPreferences("device_profile",Context.MODE_PRIVATE).edit().putString("device_id",id).remove("name").commit())throw new Exception("Cannot save device ID");
  Native.request(new JSONObject().put("op","set_device_id").put("device_id",id));
 }
 static synchronized boolean syncHost(Context context,String peer){
  try{HostStore hosts=new HostStore(context);JSONObject record=hosts.all().optJSONObject(peer);if(record==null)return true;
   String current=name(context);if(current.equals(record.optString("synced_name")))return true;
   Native.ensure(context);Native.request(new JSONObject().put("op","set_name").put("host",peer).put("name",current));
   hosts.nameSynced(peer,current);return current.equals(name(context));
  }catch(Exception ignored){return false;}
 }
 static boolean syncAll(Context context){
  boolean complete=true;try{JSONObject hosts=new HostStore(context).all();for(java.util.Iterator<String> it=hosts.keys();it.hasNext();)complete=syncHost(context,it.next())&&complete;}catch(Exception ignored){complete=false;}return complete;
 }
 private DeviceProfile(){}
}
