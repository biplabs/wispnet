package com.biplabs.wispnet;
import android.content.*;
import org.json.*;
final class HostStore {
 // Activities can overlap during recreation while their catalog workers finish.
 private static final Object LOCK=new Object();
 final SharedPreferences prefs;
 HostStore(Context context){prefs=context.getSharedPreferences("hosts",Context.MODE_PRIVATE);}
 JSONObject all()throws JSONException{synchronized(LOCK){return new JSONObject(prefs.getString("records","{}"));}}
 void save(JSONObject records)throws Exception{synchronized(LOCK){if(records.length()>128||!prefs.edit().putString("records",records.toString()).commit())throw new Exception("Cannot save host directory");}}
 void paired(String host,String name)throws Exception{synchronized(LOCK){JSONObject records=all();records.put(host,new JSONObject().put("name",name).put("state","approval_pending").put("status","Pending approval on this host"));save(records);}}
 void catalog(String host,JSONObject snapshot)throws Exception{synchronized(LOCK){
  if(!host.equals(snapshot.getString("host")))throw new Exception("Wrong catalog owner");
  JSONObject records=all(),record=records.getJSONObject(host),previous=record.optJSONObject("catalog");
  if(previous!=null){
   if(!previous.getString("generation").equals(snapshot.getString("generation")))throw new Exception("Host state changed; re-pair required");
   long old=previous.getLong("revision"),revision=snapshot.getLong("revision");
   if(revision<old||(revision==old&&!previous.getJSONArray("services").toString().equals(snapshot.getJSONArray("services").toString())))throw new Exception("Catalog rollback or inconsistent snapshot");
  }
  record.put("catalog",snapshot).put("fetched",System.currentTimeMillis()).put("state","available").put("status","Available");save(records);
 }}
 void failed(String host,Exception failure)throws Exception{synchronized(LOCK){
  JSONObject records=all();if(!records.has(host))return;JSONObject record=records.getJSONObject(host);
  String code=failure instanceof Native.Failure?((Native.Failure)failure).code:"operation_failed";
  if("host_denied".equals(code)){
   boolean pending="approval_pending".equals(record.optString("state"))||record.optString("status").startsWith("Pending approval");
   record.put("state",pending?"approval_pending":"host_denied").put("status",pending?"Pending approval on this host":"Access denied by this host. Request pairing again.");
  }else if("service_denied".equals(code)){record.put("status","Service no longer listed or granted by this host. Refresh services.");}
  else if("Host state changed; re-pair required".equals(failure.getMessage())){record.put("state","state_changed").put("status","Host state changed. Verify this host and request pairing again.");}
  else if("Catalog rollback or inconsistent snapshot".equals(failure.getMessage())||"Wrong catalog owner".equals(failure.getMessage())){record.put("state","unverified").put("status","Host catalog rejected. Cached entries are not trusted.");}
  else{
   String prior=record.optString("state");
   if("approval_pending".equals(prior))record.put("status","Pending approval on this host; host currently unavailable.");
   else if("host_denied".equals(prior))record.put("status","Access previously denied by this host; host currently unavailable.");
   else record.put("state","unreachable").put("status","Host unavailable or could not verify its response. Cached listings do not grant access.");
  }
  save(records);
 }}
}
