package com.biplabs.wispnet;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.os.SystemClock;
import android.provider.DocumentsContract;
import android.view.accessibility.AccessibilityNodeInfo;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import org.json.JSONArray;
import org.json.JSONObject;

/** Disposable-emulator checks for independent named SAF receiving services. */
final class AndroidInboxChecks {
 private final Instrumentation test;
 private final Bundle arguments;
 private NetworkBridge bridge;
 private final JSONArray checks=new JSONArray();
 AndroidInboxChecks(Instrumentation test,Bundle arguments){this.test=test;this.arguments=new Bundle(arguments);}
 void run(){
  JSONObject report=new JSONObject();Bundle result=new Bundle();
  try{
   Context context=test.getTargetContext();String host=arguments.getString("host");
   Activity activity=test.startActivitySync(new Intent(context,FlutterMainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
   context.startForegroundService(new Intent(context,AccessService.class));bridge=new NetworkBridge(context);
   request("setAccess",new JSONObject().put("enabled",true));request("pair",new JSONObject().put("code",arguments.getString("invitation")));
   JSONObject remoteInbox=null;
   for(int attempt=0;attempt<100&&remoteInbox==null;attempt++){
    request("refreshHost",new JSONObject().put("host",host));
    JSONObject record=request("snapshot",new JSONObject()).getJSONObject("hosts").getJSONObject(host);
    JSONObject catalog=record.optJSONObject("catalog");
    if(catalog!=null){JSONArray services=catalog.getJSONArray("services");for(int n=0;n<services.length();n++)if("desktop-inbox".equals(services.getJSONObject(n).optString("id")))remoteInbox=services.getJSONObject(n);}
    if(remoteInbox==null)Thread.sleep(100);
   }
   require(remoteInbox!=null,"Desktop named inbox did not become available");
   JSONObject initial=request("folderServicesStatus",new JSONObject());
   require(initial.getJSONArray("inboxes").length()==0&&initial.getJSONArray("shares").length()==0,"Use a freshly cleared disposable app fixture");
   JSONObject legacy=inbox(request("receivingStatus",new JSONObject()));
   Uri first=tree(arguments.getString("receiving_a")),second=tree(arguments.getString("receiving_b"));
   JSONObject a=pick(activity,first),b=pick(activity,second);
   JSONObject config=new JSONObject().put("id","inbox-a").put("folder",a.getString("folder")).put("allow",new JSONArray().put(host)).put("expected_revision",initial.getLong("revision"));
   request("saveInbox",config);
   boolean stale=false;try{request("saveInbox",config);}catch(Exception conflict){stale=true;}require(stale,"Stale inbox edit accepted");
   JSONObject status=request("folderServicesStatus",new JSONObject());
   request("saveInbox",new JSONObject().put("id","inbox-b").put("folder",b.getString("folder")).put("allow",new JSONArray()).put("expected_revision",status.getLong("revision")));
   status=request("folderServicesStatus",new JSONObject());require(status.getJSONArray("inboxes").length()==2,"Named receiving folders missing");
   checks.put("two writable SAF roots and independent per-inbox grants");
   String peer=request("snapshot",new JSONObject()).getString("peer");test.runOnMainSync(()->activity.moveTaskToBack(true));
   signal("first",peer);JSONObject firstReceipt=await("first");require(firstReceipt.getBoolean("success"),firstReceipt.toString());
   verify(first,"same.bin",firstReceipt.getJSONObject("file"));
   require(!exists(second,"same.bin"),"Denied inbox received data");
   checks.put("desktop native upload enters only selected permitted inbox");
   status=request("folderServicesStatus",new JSONObject());
   request("saveInbox",new JSONObject().put("id","inbox-b").put("folder",b.getString("folder")).put("allow",new JSONArray().put(host)).put("expected_revision",status.getLong("revision")));
   signal("both",peer);JSONObject both=await("both");require(both.getBoolean("success"),both.toString());
   verify(first,"same.bin",firstReceipt.getJSONObject("file"));verify(second,"same.bin",both.getJSONObject("file"));
   checks.put("identical destination names remain separate across receiving folders");
   status=request("folderServicesStatus",new JSONObject());
   request("saveInbox",new JSONObject().put("id","inbox-a").put("folder",a.getString("folder")).put("allow",new JSONArray()).put("expected_revision",status.getLong("revision")));
   signal("revoked",peer);JSONObject revoked=await("revoked");require(revoked.getBoolean("success"),revoked.toString());
   verify(second,"still.bin",revoked.getJSONObject("file"));require(!exists(first,"revoked.bin"),"Revoked inbox received a file");
   checks.put("revoking one inbox leaves the other usable and prior files intact");
   // Android uploads also retain the selected inbox id through its native job.
   Uri source=FilesTestProvider.fixture("direct",1024*1024+7,57);
   context.getContentResolver().call(Uri.parse("content://"+FilesTestProvider.AUTHORITY),"grant",source.toString(),null);
   JSONObject job=request("fileSend",new JSONObject().put("host",host).put("service","desktop-inbox").put("incarnation",remoteInbox.getString("incarnation"))
       .put("workspace","").put("local_path",source.toString()).put("remote_path","android-to-named.bin"));
   long until=SystemClock.elapsedRealtime()+90000;
   while("running".equals(job.optString("state"))&&SystemClock.elapsedRealtime()<until){Thread.sleep(50);job=request("fileStatus",new JSONObject().put("transfer",job.getString("transfer")));}
   require("complete".equals(job.optString("state")),"Android named-inbox upload failed: "+job);
   require("desktop-inbox".equals(job.getString("service")),"Android job lost its selected inbox id");
   require(FilesTestProvider.expectedSha256(1024*1024+7,57).equals(job.getJSONObject("file").getString("sha256")),"Android outbound checksum changed");
   checks.put("Android sender and restored job metadata retain selected named inbox");
   status=request("folderServicesStatus",new JSONObject());require(status.getJSONArray("shares").length()==0,"Receiving edits changed source shares");
   require(legacy.toString().equals(inbox(request("receivingStatus",new JSONObject())).toString()),"Named inboxes changed legacy receiving defaults");
   for(String id:new String[]{"inbox-a","inbox-b"}){status=request("folderServicesStatus",new JSONObject());request("removeInbox",new JSONObject().put("id",id).put("expected_revision",status.getLong("revision")));}
   verify(first,"same.bin",firstReceipt.getJSONObject("file"));verify(second,"same.bin",both.getJSONObject("file"));
   checks.put("source shares and legacy defaults unchanged; removal preserves received files");
   report.put("success",true).put("checks",checks).put("outbound",job.getJSONObject("file"));
   test.finish(Activity.RESULT_OK,save(report,result));
  }catch(Throwable failure){try{report.put("success",false).put("error",failure.toString()).put("checks",checks);}catch(Exception ignored){}test.finish(Activity.RESULT_CANCELED,save(report,result));}
  finally{if(bridge!=null)bridge.close();}
 }
 private JSONObject request(String method,JSONObject args)throws Exception{return bridge.request(method,args);}
 private static JSONObject inbox(JSONObject status)throws Exception{return new JSONObject().put("folder",status.getString("folder")).put("allow",status.getJSONArray("allow"));}
 private Uri tree(String name)throws Exception{require(name!=null&&name.matches("WispnetInboxTest-[0-9a-f]{12}-[ab]"),"Use disposable fixture roots");return DocumentsContract.buildTreeDocumentUri("com.android.externalstorage.documents","primary:Documents/"+name);}
 private JSONObject pick(Activity activity,Uri tree)throws Exception{
  Intent picker=new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION|Intent.FLAG_GRANT_WRITE_URI_PERMISSION|Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION|Intent.FLAG_GRANT_PREFIX_URI_PERMISSION)
      .putExtra(DocumentsContract.EXTRA_INITIAL_URI,DocumentsContract.buildDocumentUriUsingTree(tree,DocumentsContract.getTreeDocumentId(tree)));
  test.runOnMainSync(()->activity.startActivityForResult(picker,106));click("Use this folder",20000);click("Allow",10000);Thread.sleep(300);
  test.getTargetContext().getContentResolver().takePersistableUriPermission(tree,Intent.FLAG_GRANT_READ_URI_PERMISSION|Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
  return request("prepareReceivingFolder",new JSONObject().put("uri",tree.toString()));
 }
 private void verify(Uri tree,String path,JSONObject expected)throws Exception{
  Uri file=DocumentsContract.buildDocumentUriUsingTree(tree,DocumentsContract.getTreeDocumentId(tree)+"/"+path);MessageDigest digest=MessageDigest.getInstance("SHA-256");long count=0;
  try(InputStream input=test.getTargetContext().getContentResolver().openInputStream(file)){require(input!=null,"Missing received file");byte[] buffer=new byte[65536];int read;while((read=input.read(buffer))!=-1){count+=read;digest.update(buffer,0,read);}}
  require(count==expected.getLong("size_bytes")&&java.util.HexFormat.of().formatHex(digest.digest()).equals(expected.getString("sha256")),"Wrong receiving root or content: "+path);
 }
 private boolean exists(Uri tree,String name)throws Exception{
  Uri children=DocumentsContract.buildChildDocumentsUriUsingTree(tree,DocumentsContract.getTreeDocumentId(tree));
  try(android.database.Cursor rows=test.getTargetContext().getContentResolver().query(children,new String[]{DocumentsContract.Document.COLUMN_DISPLAY_NAME},null,null,null)){
   require(rows!=null,"Receiving folder listing unavailable");
   int count=0;while(rows.moveToNext()){require(++count<=100000,"Receiving folder listing exceeded fixture bound");if(name.equals(rows.getString(0)))return true;}
   return false;
  }
 }

 private void signal(String phase,String peer)throws Exception{try(FileOutputStream out=new FileOutputStream(new File(test.getTargetContext().getCacheDir(),"android-inboxes-ready.json"))){out.write(new JSONObject().put("phase",phase).put("peer",peer).toString().getBytes(StandardCharsets.UTF_8));}}
 private JSONObject await(String phase)throws Exception{File file=new File(test.getTargetContext().getCacheDir(),"android-inboxes-host-"+phase+".json");long until=SystemClock.elapsedRealtime()+120000;while(SystemClock.elapsedRealtime()<until){if(file.isFile())try(FileInputStream input=new FileInputStream(file)){return new JSONObject(new String(input.readAllBytes(),StandardCharsets.UTF_8));}Thread.sleep(100);}throw new Exception("Desktop fixture timed out: "+phase);}
 private void click(String label,long timeout)throws Exception{long until=SystemClock.elapsedRealtime()+timeout;while(SystemClock.elapsedRealtime()<until){AccessibilityNodeInfo root=test.getUiAutomation().getRootInActiveWindow();if(root!=null){java.util.ArrayDeque<AccessibilityNodeInfo> nodes=new java.util.ArrayDeque<>();nodes.add(root);while(!nodes.isEmpty()){AccessibilityNodeInfo node=nodes.removeFirst();CharSequence text=node.getText();if(text!=null&&label.equalsIgnoreCase(text.toString())&&node.isEnabled()){AccessibilityNodeInfo action=node;for(int level=0;level<3&&action!=null;level++,action=action.getParent())if(action.performAction(AccessibilityNodeInfo.ACTION_CLICK))return;}for(int child=0;child<node.getChildCount();child++){AccessibilityNodeInfo next=node.getChild(child);if(next!=null)nodes.add(next);}}}Thread.sleep(150);}throw new Exception("System picker missing: "+label);}
 private Bundle save(JSONObject report,Bundle result){try(FileOutputStream out=new FileOutputStream(new File(test.getTargetContext().getCacheDir(),"android-inboxes-result.json"))){out.write(report.toString().getBytes(StandardCharsets.UTF_8));result.putString("result",report.toString());}catch(Exception failure){result.putString("error",failure.toString());}return result;}
 private static void require(boolean value,String message)throws Exception{if(!value)throw new Exception(message);}
}
