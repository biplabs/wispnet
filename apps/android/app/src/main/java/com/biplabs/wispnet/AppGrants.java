package com.biplabs.wispnet;

import android.content.*;
import android.content.pm.*;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import org.json.*;

/** Grants bind an OS caller, signing certificate, host, service and incarnation. */
final class AppGrants {
 static final Object LOCK=new Object();
 private final Context context;
 AppGrants(Context context){this.context=context;}
 static String hash(String value)throws Exception{
  byte[] bytes=MessageDigest.getInstance("SHA-256").digest(value.getBytes(StandardCharsets.UTF_8));
  StringBuilder out=new StringBuilder();for(byte b:bytes)out.append(String.format(java.util.Locale.ROOT,"%02x",b&255));return out.toString();
 }
 static String identity(Context context,int uid,String pkg)throws Exception{
  PackageManager pm=context.getPackageManager();String[] packages=pm.getPackagesForUid(uid);
  if(packages==null||packages.length!=1||!packages[0].equals(pkg)||pm.getPackageUid(pkg,0)!=uid)throw new SecurityException("Caller identity unavailable or shared UID unsupported");
  PackageInfo info=pm.getPackageInfo(pkg,PackageManager.GET_SIGNING_CERTIFICATES);
  Signature[] signatures=info.signingInfo.getApkContentsSigners();
  if(signatures.length!=1)throw new SecurityException("Multiple signers unsupported");
  return hash(pkg+":"+android.util.Base64.encodeToString(signatures[0].toByteArray(),android.util.Base64.NO_WRAP));
 }
 JSONObject all()throws Exception{return new JSONObject(context.getSharedPreferences("app_access",0).getString("grants","{}"));}
 void save(JSONObject grants)throws Exception{
  if(grants.length()>128||!context.getSharedPreferences("app_access",0).edit().putString("grants",grants.toString()).commit())throw new Exception("Cannot save app access");
 }
 static String key(String identity,String host,String service,String incarnation)throws Exception{return hash(identity+":"+host+":"+service+":"+incarnation);}
 void approve(String id,JSONObject grant)throws Exception{synchronized(LOCK){JSONObject all=all();all.put(id,grant);save(all);}}
 void revoke(String id)throws Exception{synchronized(LOCK){
  // Persist denial before touching native state, so a retry cannot renew a grant.
  JSONObject all=all();all.remove(id);save(all);
  if(Native.ready)try{Native.request(new JSONObject().put("op","app_revoke").put("grant",id));}
  catch(Exception failure){context.stopService(new Intent(context,AccessService.class));throw failure;}
 }}
}
