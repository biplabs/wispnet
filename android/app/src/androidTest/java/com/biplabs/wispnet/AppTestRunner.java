package com.biplabs.wispnet;
import android.app.*;
import android.content.*;
import android.os.*;
import android.view.*;
import android.widget.*;
import android.view.accessibility.AccessibilityNodeInfo;
import java.nio.file.Files;
import java.io.File;
import org.json.*;

/** Debug instrumentation drives the production UI; no exported test endpoints. */
public final class AppTestRunner extends Instrumentation {
 Bundle args;MainActivity activity;
 public void onCreate(Bundle arguments){super.onCreate(arguments);args=arguments;start();}
 interface Check{boolean ok()throws Exception;}
 void await(Check check,String message)throws Exception{long end=System.currentTimeMillis()+45000;while(System.currentTimeMillis()<end){if(check.ok())return;Thread.sleep(200);}throw new Exception(message);}
 void stage(String name)throws Exception{Bundle state=new Bundle();state.putString("stage",name);state.putString("peer",Native.command("identity").getString("peer"));sendStatus(0,state);}
 Button find(View view,String title){if(view instanceof Button&&((Button)view).getText().toString().equals(title))return(Button)view;if(view instanceof ViewGroup){ViewGroup group=(ViewGroup)view;for(int i=0;i<group.getChildCount();i++){Button b=find(group.getChildAt(i),title);if(b!=null)return b;}}return null;}
 void click(String title)throws Exception{final boolean[] found={false};runOnMainSync(()->{Button b=find(activity.root,title);if(b!=null){found[0]=true;b.performClick();}});if(!found[0])throw new Exception("Missing UI control: "+title);}
 boolean text(AccessibilityNodeInfo node,String needle){if(node==null)return false;if(node.getText()!=null&&node.getText().toString().contains(needle))return true;if(node.getContentDescription()!=null&&node.getContentDescription().toString().contains(needle))return true;for(int i=0;i<node.getChildCount();i++){if(text(node.getChild(i),needle))return true;}return false;}
 boolean browserText(String needle){AccessibilityNodeInfo root=getUiAutomation().getRootInActiveWindow();return root!=null&&"com.android.chrome".contentEquals(root.getPackageName())&&text(root,needle);}
 String http(UriParts url,String path,String method,String extra,String body)throws Exception{
  try(java.net.Socket socket=new java.net.Socket("127.0.0.1",url.port)){
   socket.setSoTimeout(45000);String request=method+" "+path+" HTTP/1.1\r\nHost: "+url.authority+"\r\nConnection: close\r\nContent-Length: "+body.getBytes(java.nio.charset.StandardCharsets.UTF_8).length+"\r\n"+extra+"\r\n"+body;
   socket.getOutputStream().write(request.getBytes(java.nio.charset.StandardCharsets.UTF_8));return new String(socket.getInputStream().readNBytes(1024*1024),java.nio.charset.StandardCharsets.UTF_8);
  }
 }
 static class UriParts {final String authority,token;final int port;UriParts(String url){android.net.Uri uri=android.net.Uri.parse(url);authority=uri.getAuthority();port=uri.getPort();token=uri.getFragment();}}
 UriParts protectedRequestChecks(String host,String service)throws Exception{
  UriParts url=new UriParts(Native.request(new JSONObject().put("op","open").put("host",host).put("service",service)).getString("url"));
  if(!http(url,"/","GET","","").startsWith("HTTP/1.1 401"))throw new Exception("Missing browser session accepted");
  if(!http(url,"/","GET","Origin: https://attacker.invalid\r\n","").startsWith("HTTP/1.1 403"))throw new Exception("Cross-origin browser request accepted");
  String exchange="Origin: http://"+url.authority+"\r\nAuthorization: Bearer "+url.token+"\r\n";
  String response=http(url,"/_wipsnet/session","POST",exchange,"");if(!response.startsWith("HTTP/1.1 204"))throw new Exception("Native gateway bootstrap failed");
  String cookie=null;for(String line:response.split("\r\n"))if(line.startsWith("Set-Cookie: "))cookie=line.substring(12).split(";",2)[0];
  if(cookie==null)throw new Exception("Gateway session cookie missing");
  if(!http(url,"/_wipsnet/session","POST",exchange,"").startsWith("HTTP/1.1 403"))throw new Exception("Bootstrap was reusable");
  String body="{\"messages\":[{\"role\":\"user\",\"content\":\"Reply with OK.\"}],\"max_tokens\":32,\"stream\":true,\"chat_template_kwargs\":{\"enable_thinking\":false}}";
  String streamed=http(url,"/v1/chat/completions","POST","Origin: http://"+url.authority+"\r\nCookie: "+cookie+"\r\nContent-Type: application/json\r\n",body);
  if(!streamed.contains("data:")||!streamed.contains("[DONE]"))throw new Exception("Actual Llama streaming reply failed");
  return url;
 }
 void resume(){Intent intent=getTargetContext().getPackageManager().getLaunchIntentForPackage(getTargetContext().getPackageName());intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK|Intent.FLAG_ACTIVITY_CLEAR_TOP);activity=(MainActivity)startActivitySync(intent);waitForIdleSync();}
 public void onStart(){Bundle result=new Bundle();try{
  if(args.getString("mode","").startsWith("peers-")){PeerChecks.run(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  if("lifecycle-cold".equals(args.getString("mode"))){LifecycleChecks.cold(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  if("lifecycle".equals(args.getString("mode"))){LifecycleChecks.run(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  resume();Native.ensure(getTargetContext());stage("ready");String[] hosts={args.getString("host_a"),args.getString("host_b")};String[] names={"Local computer","Desktop"};
  if(!"resume".equals(args.getString("mode"))){
   for(int i=0;i<2;i++){final String host=hosts[i],name=names[i];String document=new String(Files.readAllBytes(new File(getTargetContext().getFilesDir(),"invitation-"+i+".json").toPath()),java.nio.charset.StandardCharsets.UTF_8);
    runOnMainSync(()->{activity.label.setText(name);activity.host.setText(host);activity.invitation.setText(document);});click("Request pairing");await(()->new HostStore(getTargetContext()).all().has(host),"Pairing UI failed");new File(getTargetContext().getFilesDir(),"invitation-"+i+".json").delete();stage("paired-"+i);
   }
  }
  await(()->{runOnMainSync(()->activity.refresh());JSONObject all=new HostStore(getTargetContext()).all();return all.has(hosts[0])&&all.has(hosts[1])&&"Available".equals(all.getJSONObject(hosts[0]).optString("status"))&&"Available".equals(all.getJSONObject(hosts[1]).optString("status"));},"Approved catalogs did not appear independently");stage("catalogs");
  click("Start browser access");await(()->AccessService.enabled,"Foreground access did not start");
  click("Open Local computer / website");await(()->browserText("Follow the evidence"),"GIKE did not load in the normal browser");stage("gike-browser");
  resume();click("Open Desktop / website");await(()->browserText("WispShell"),"Desktop website did not load in browser");stage("desktop-browser");
  resume();click("Open Local computer / llama");await(()->browserText("New chat")&&browserText("Hello there"),"Llama UI did not load in browser");stage("llama-browser");
  UriParts protectedUrl=protectedRequestChecks(hosts[0],"llama");stage("gateway-denials-and-llama-stream");
  // Ordinary browser remains foreground while the native foreground service owns access.
  Thread.sleep(30000);if(!AccessService.enabled)throw new Exception("Browser handoff lost foreground access");stage("background-30s");
  resume();click("Stop browser access");await(()->!AccessService.enabled,"Stop did not stop foreground access");stage("stopped");
  Thread.sleep(300);try(java.net.Socket unexpected=new java.net.Socket("127.0.0.1",protectedUrl.port)){throw new Exception("Stop left a gateway listener open");}catch(java.net.ConnectException expected){}
  result.putString("result","passed");finish(Activity.RESULT_OK,result);
 }catch(Throwable error){if(activity!=null)runOnMainSync(()->result.putString("ui_status",activity.status.getText().toString()));result.putString("result","failed");result.putString("failure",error.getMessage()==null?error.getClass().getSimpleName():error.getMessage());finish(Activity.RESULT_CANCELED,result);}}
}
