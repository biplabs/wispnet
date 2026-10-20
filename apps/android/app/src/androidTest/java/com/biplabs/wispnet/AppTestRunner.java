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
 Button find(View view,String title){if(view instanceof Button&&(((Button)view).getText().toString().equals(title)||title.equals(view.getTag())))return(Button)view;if(view instanceof ViewGroup){ViewGroup group=(ViewGroup)view;for(int i=0;i<group.getChildCount();i++){Button b=find(group.getChildAt(i),title);if(b!=null)return b;}}return null;}
 void click(String title)throws Exception{if(title.startsWith("Open "))await(()->activity.spaceShown&&!activity.jumping,"Services not visible");final boolean[] found={false};runOnMainSync(()->{Button b=java.util.List.of("Start browser access","Stop browser access").contains(title)?activity.access:find(activity.root,title);if(b!=null){found[0]=true;b.performClick();}});if(!found[0])throw new Exception("Missing UI control: "+title);}
 boolean text(AccessibilityNodeInfo node,String needle){if(node==null)return false;if(node.getText()!=null&&node.getText().toString().contains(needle))return true;if(node.getContentDescription()!=null&&node.getContentDescription().toString().contains(needle))return true;for(int i=0;i<node.getChildCount();i++){if(text(node.getChild(i),needle))return true;}return false;}
 boolean browserText(String needle){AccessibilityNodeInfo root=getUiAutomation().getRootInActiveWindow();return root!=null&&"com.android.chrome".contentEquals(root.getPackageName())&&text(root,needle);}
 String http(UriParts url,String path,String method,String extra,String body)throws Exception{
  try(java.net.Socket socket=new java.net.Socket("127.0.0.1",url.port)){
   socket.setSoTimeout(45000);String request=method+" "+path+" HTTP/1.1\r\nHost: "+url.authority+"\r\nConnection: close\r\nContent-Length: "+body.getBytes(java.nio.charset.StandardCharsets.UTF_8).length+"\r\n"+extra+"\r\n"+body;
   socket.getOutputStream().write(request.getBytes(java.nio.charset.StandardCharsets.UTF_8));return new String(socket.getInputStream().readNBytes(1024*1024),java.nio.charset.StandardCharsets.UTF_8);
  }
 }
 static class UriParts {final String authority,token;final int port;UriParts(String url){android.net.Uri uri=android.net.Uri.parse(url);authority=uri.getAuthority();port=uri.getPort();token=uri.getFragment();}}
 String incarnation(String host,String service)throws Exception{org.json.JSONArray services=Native.request(new JSONObject().put("op","catalog").put("host",host)).getJSONArray("services");for(int i=0;i<services.length();i++)if(service.equals(services.getJSONObject(i).getString("id")))return services.getJSONObject(i).getString("incarnation");throw new Exception("Service unavailable");}
 UriParts protectedRequestChecks(String host,String service)throws Exception{
  UriParts url=new UriParts(Native.request(new JSONObject().put("op","open").put("host",host).put("service",service).put("incarnation",incarnation(host,service))).getString("url"));
  if(!http(url,"/","GET","Sec-Fetch-Site: same-origin\r\n","").startsWith("HTTP/1.1 401"))throw new Exception("Missing browser session accepted");
  if(!http(url,"/","GET","Origin: https://attacker.invalid\r\n","").startsWith("HTTP/1.1 403"))throw new Exception("Cross-origin browser request accepted");
  String response=http(url,"/","GET","Sec-Fetch-Site: none\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: document\r\n","");if(!response.startsWith("HTTP/1.1 303"))throw new Exception("Browser navigation authorization failed");
  String cookie=null;for(String line:response.split("\r\n"))if(line.startsWith("Set-Cookie: "))cookie=line.substring(12).split(";",2)[0];
  if(cookie==null)throw new Exception("Gateway session cookie missing");
  String body="{\"messages\":[{\"role\":\"user\",\"content\":\"Reply with OK.\"}],\"max_tokens\":32,\"stream\":true,\"chat_template_kwargs\":{\"enable_thinking\":false}}";
  String streamed=http(url,"/v1/chat/completions","POST","Origin: http://"+url.authority+"\r\nCookie: "+cookie+"\r\nContent-Type: application/json\r\n",body);
  if(!streamed.contains("data:")||!streamed.contains("[DONE]"))throw new Exception("Actual Llama streaming reply failed");
  return url;
 }
 void resume(){Intent intent=getTargetContext().getPackageManager().getLaunchIntentForPackage(getTargetContext().getPackageName());intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK|Intent.FLAG_ACTIVITY_CLEAR_TOP);activity=(MainActivity)startActivitySync(intent);waitForIdleSync();}
 public void onStart(){Bundle result=new Bundle();try{
  if("removal-identity".equals(args.getString("mode"))){Native.ensure(getTargetContext());result.putString("peer",Native.command("identity").getString("peer"));result.putString("hosts",new HostStore(getTargetContext()).all().toString());result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  if(args.getString("mode","").startsWith("removal-")){RemovalChecks.run(this);result.putString("peer",Native.command("identity").getString("peer"));result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}

  if("service-launch".equals(args.getString("mode"))){SpaceChecks.serviceLaunch(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  if("discovery".equals(args.getString("mode"))){DiscoveryChecks.run(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  if("header-layout".equals(args.getString("mode"))){SpaceChecks.headerLayout(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  if("network-feedback".equals(args.getString("mode"))){SpaceChecks.networkFeedback(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  if("button-layout".equals(args.getString("mode"))){SpaceChecks.buttonLayout(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  if("space-ui".equals(args.getString("mode"))){SpaceChecks.run(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  if(java.util.List.of("qr-auto-pair","code-pair").contains(args.getString("mode",""))){AutoPairChecks.run(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  if("scanner-launch".equals(args.getString("mode"))){ScannerChecks.run(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  if("platform-timeout".equals(args.getString("mode"))){PlatformChecks.run(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  if(args.getString("mode","").startsWith("peers-")){if(java.util.List.of("peers-repair","peers-expiry","peers-egress").contains(args.getString("mode")))AcceptanceChecks.run(this);else PeerChecks.run(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  if("lifecycle-cold".equals(args.getString("mode"))){LifecycleChecks.cold(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  if("lifecycle".equals(args.getString("mode"))){LifecycleChecks.run(this);result.putString("result","passed");finish(Activity.RESULT_OK,result);return;}
  resume();Native.ensure(getTargetContext());stage("ready");String[] hosts={args.getString("host_a"),args.getString("host_b")};String[] names={"Local computer","Desktop"};
  if(!"resume".equals(args.getString("mode"))){
   for(int i=0;i<2;i++){final String host=hosts[i],name=names[i];String document=new String(Files.readAllBytes(new File(getTargetContext().getFilesDir(),"invitation-"+i+".json").toPath()),java.nio.charset.StandardCharsets.UTF_8);
    runOnMainSync(()->activity.reviewInvitation("wispnet1:"+android.util.Base64.encodeToString(document.getBytes(java.nio.charset.StandardCharsets.UTF_8),android.util.Base64.URL_SAFE|android.util.Base64.NO_PADDING|android.util.Base64.NO_WRAP)));await(()->activity.status.getText().toString().contains("Pairing code ready"),"Pairing code review failed");runOnMainSync(()->activity.label.setText(name));click("Request pairing");await(()->new HostStore(getTargetContext()).all().has(host),"Pairing UI failed");new File(getTargetContext().getFilesDir(),"invitation-"+i+".json").delete();stage("paired-"+i);
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
