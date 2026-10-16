package com.biplabs.wispnet;
import android.content.*;
import org.json.*;
import java.util.*;
final class HostStore {
 final SharedPreferences prefs;
 HostStore(Context context){prefs=context.getSharedPreferences("hosts",Context.MODE_PRIVATE);}
 synchronized JSONObject all()throws JSONException{return new JSONObject(prefs.getString("records","{}"));}
 synchronized void save(JSONObject records)throws Exception{if(records.length()>128||!prefs.edit().putString("records",records.toString()).commit())throw new Exception("Cannot save host directory");}
 synchronized void paired(String host,String name)throws Exception{JSONObject records=all();records.put(host,new JSONObject().put("name",name).put("status","Pending approval on this host"));save(records);}
 synchronized void catalog(String host,JSONObject snapshot)throws Exception{
  if(!host.equals(snapshot.getString("host")))throw new Exception("Wrong catalog owner");
  JSONObject records=all(),record=records.getJSONObject(host),previous=record.optJSONObject("catalog");
  if(previous!=null){
   if(!previous.getString("generation").equals(snapshot.getString("generation")))throw new Exception("Host state changed; re-pair required");
   long old=previous.getLong("revision"),revision=snapshot.getLong("revision");
   if(revision<old||(revision==old&&!previous.getJSONArray("services").toString().equals(snapshot.getJSONArray("services").toString())))throw new Exception("Catalog rollback or inconsistent snapshot");
  }
  record.put("catalog",snapshot).put("fetched",System.currentTimeMillis()).put("status","Available");save(records);
 }
 synchronized void failed(String host,String status)throws Exception{JSONObject records=all();if(records.has(host)){records.getJSONObject(host).put("status",status);save(records);}}
}
