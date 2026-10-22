package com.biplabs.wispnet;
import android.content.*;
import android.os.Build;
import android.provider.Settings;
import java.nio.charset.StandardCharsets;
import org.json.*;

/** Display metadata, independent of the protected device key and browser addresses. */
final class DeviceProfile {
 static boolean valid(String name){return name!=null&&!name.codePoints().allMatch(cp->Character.isWhitespace(cp)||Character.isSpaceChar(cp))&&StandardCharsets.UTF_8.newEncoder().canEncode(name)&&name.getBytes(StandardCharsets.UTF_8).length<=128&&name.codePoints().noneMatch(Character::isISOControl);}
 static String name(Context context){
  String custom=context.getSharedPreferences("device_profile",Context.MODE_PRIVATE).getString("name",null);
  if(valid(custom))return custom;
  String system=null;try{system=Settings.Global.getString(context.getContentResolver(),"device_name");}catch(SecurityException ignored){}
  if(system==null||system.trim().isEmpty())system=Build.MODEL;
  if(system==null)system="Android device";
  StringBuilder clean=new StringBuilder();
  for(int cp:system.trim().codePoints().toArray()){if(Character.isISOControl(cp))continue;String next=new String(Character.toChars(cp));if((clean.toString()+next).getBytes(StandardCharsets.UTF_8).length>128)break;clean.append(next);}
  return valid(clean.toString())?clean.toString():"Android device";
 }
 static void save(Context context,String name)throws Exception{
  if(!valid(name))throw new Exception("Use 1–128 bytes, without control characters");
  if(!context.getSharedPreferences("device_profile",Context.MODE_PRIVATE).edit().putString("name",name).commit())throw new Exception("Cannot save device name");
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
