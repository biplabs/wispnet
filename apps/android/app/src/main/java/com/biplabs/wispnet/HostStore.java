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
 void remove(String host)throws Exception{synchronized(LOCK){JSONObject records=all();records.remove(host);save(records);}}
 static void validate(String host,JSONObject c)throws Exception{
  if(c.length()!=7||!Integer.valueOf(1).equals(c.get("version"))||!Boolean.TRUE.equals(c.get("complete"))||!host.equals(c.getString("host"))||!c.getString("generation").matches("[0-9a-f]{64}"))throw new Exception("Invalid or incomplete catalog");
  Object rev=c.get("revision"),count=c.get("service_count");
  if(!(rev instanceof Integer||rev instanceof Long)||c.getLong("revision")<0||!(count instanceof Integer))throw new Exception("Invalid or incomplete catalog");
  JSONArray services=c.getJSONArray("services");if(services.length()>128||c.getInt("service_count")!=services.length())throw new Exception("Invalid or incomplete catalog");
  String previous="";for(int i=0;i<services.length();i++){
   JSONObject r=services.getJSONObject(i);for(String key:new String[]{"id","incarnation","display_name","kind"})if(!(r.get(key) instanceof String))throw new Exception("Invalid or incomplete catalog");String id=r.getString("id"),name=r.getString("display_name"),kind=r.getString("kind");
   if(r.length()!=4||!id.matches("[a-z0-9][a-z0-9._-]{0,63}")||id.compareTo(previous)<=0||!r.getString("incarnation").matches("[0-9a-f]{64}")||name.codePoints().allMatch(cp->Character.isWhitespace(cp)||Character.isSpaceChar(cp))||!java.nio.charset.StandardCharsets.UTF_8.newEncoder().canEncode(name)||name.getBytes(java.nio.charset.StandardCharsets.UTF_8).length>128||!("http".equals(kind)||"tcp".equals(kind)))throw new Exception("Invalid or incomplete catalog");
   for(int j=0;j<name.length();j++)if(Character.isISOControl(name.charAt(j)))throw new Exception("Invalid or incomplete catalog");previous=id;
  }
 }
 void catalog(String host,JSONObject snapshot)throws Exception{synchronized(LOCK){
  validate(host,snapshot);
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
  else if("Catalog rollback or inconsistent snapshot".equals(failure.getMessage())||"Wrong catalog owner".equals(failure.getMessage())||"Invalid or incomplete catalog".equals(failure.getMessage())||"catalog_rejected".equals(code)){record.put("state","unverified").put("status","Host catalog rejected. Cached entries are not trusted.");}
  else{
   String prior=record.optString("state");
   if("approval_pending".equals(prior))record.put("status","Pending approval on this host; host currently unavailable.");
   else if("host_denied".equals(prior))record.put("status","Access previously denied by this host; host currently unavailable.");
   else record.put("state","unreachable").put("status","Host unavailable or could not verify its response. Cached listings do not grant access.");
  }
  save(records);
 }}
}
