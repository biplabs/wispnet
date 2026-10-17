package com.biplabs.wispnet;
import android.app.*;
import android.content.*;
import java.io.*;
import java.net.*;
import java.util.concurrent.*;
import org.json.*;

/** Physical-test protocol: public stage names, private in-process session credentials. */
final class LifecycleChecks {
 static final class Session {
  final AppTestRunner t;final AppTestRunner.UriParts url;final String cookie;
  Session(AppTestRunner runner,String host)throws Exception{t=runner;url=new AppTestRunner.UriParts(Native.request(new JSONObject().put("op","open").put("host",host).put("service",runner.args.getString("fixture")).put("incarnation",runner.incarnation(host,runner.args.getString("fixture")))).getString("url"));
   String r=t.http(url,"/_wipsnet/session","POST","Origin: http://"+url.authority+"\r\nAuthorization: Bearer "+url.token+"\r\n","");String c=null;for(String line:r.split("\r\n"))if(line.startsWith("Set-Cookie: "))c=line.substring(12).split(";",2)[0];if(c==null)throw new Exception("No lifecycle session");cookie=c;
  }
  String request(String path)throws Exception{return t.http(url,path,"GET","Cookie: "+cookie+"\r\n","");}
  Socket socket(String path,String method)throws Exception{Socket s=new Socket("127.0.0.1",url.port);s.setSoTimeout(90000);s.getOutputStream().write((method+" "+path+" HTTP/1.1\r\nHost: "+url.authority+"\r\nOrigin: http://"+url.authority+"\r\nCookie: "+cookie+"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").getBytes(java.nio.charset.StandardCharsets.US_ASCII));return s;}
 }
 static void barrier(AppTestRunner t,String name)throws Exception{
  File file=new File(t.getTargetContext().getFilesDir(),"lifecycle-continue");file.delete();t.stage(name);long end=System.currentTimeMillis()+120000;
  while(!file.exists()){if(System.currentTimeMillis()>end)throw new Exception("Timed out waiting for "+name);Thread.sleep(100);}file.delete();
 }
 static void closed(AppTestRunner.UriParts url)throws Exception{try(Socket s=new Socket("127.0.0.1",url.port)){throw new Exception("Old listener survived interruption");}catch(ConnectException expected){}}
 static Session recover(AppTestRunner t,String host)throws Exception{
  final Session[] next={null};t.await(()->{try{next[0]=new Session(t,host);return next[0].request("/").contains("lifecycle-fixture");}catch(Exception unavailable){return false;}},"Network recovery exceeded 45 seconds");return next[0];
 }
 static void websocket(Session session)throws Exception{
  try(Socket s=new Socket("127.0.0.1",session.url.port)){s.setSoTimeout(15000);OutputStream out=s.getOutputStream();InputStream in=s.getInputStream();
   out.write(("GET /ws HTTP/1.1\r\nHost: "+session.url.authority+"\r\nOrigin: http://"+session.url.authority+"\r\nCookie: "+session.cookie+"\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n").getBytes(java.nio.charset.StandardCharsets.US_ASCII));
   ByteArrayOutputStream headers=new ByteArrayOutputStream();while(headers.size()<8192){int b=in.read();if(b<0)throw new Exception("WebSocket closed before upgrade");headers.write(b);if(headers.toString("US-ASCII").endsWith("\r\n\r\n"))break;}
   if(!headers.toString("US-ASCII").startsWith("HTTP/1.1 101"))throw new Exception("WebSocket upgrade failed");
   byte[] body="hello".getBytes(java.nio.charset.StandardCharsets.US_ASCII);out.write(new byte[]{(byte)0x81,(byte)0x85,1,2,3,4});for(int i=0;i<body.length;i++)out.write(body[i]^(i%4+1));out.flush();
   if(in.read()!=0x81||in.read()!=5||!java.util.Arrays.equals(body,in.readNBytes(5)))throw new Exception("WebSocket echo failed");
  }
 }
 static void cold(AppTestRunner t)throws Exception{
  t.resume();Native.ensure(t.getTargetContext());JSONObject evidence=new JSONObject(new String(java.nio.file.Files.readAllBytes(new File(t.getTargetContext().getFilesDir(),"lifecycle-evidence.json").toPath()),java.nio.charset.StandardCharsets.UTF_8));
  if(AccessService.enabled||!evidence.getString("peer").equals(Native.command("identity").getString("peer"))||new HostStore(t.getTargetContext()).all().length()!=2)throw new Exception("Process death changed identity/pairings or restarted access");
  try(Socket socket=new Socket("127.0.0.1",evidence.getInt("port"))){throw new Exception("Process death retained listener");}catch(ConnectException expected){}
  t.click("Start browser access");t.await(()->AccessService.enabled,"Cold restart failed");Session session=recover(t,t.args.getString("host_a"));websocket(session);t.click("Stop browser access");t.await(()->!AccessService.enabled,"Cold stop failed");closed(session.url);t.stage("cold-restart-preserved-pins-and-stopped-access");
 }
 static void run(AppTestRunner t)throws Exception{
  t.resume();Native.ensure(t.getTargetContext());String peer=Native.command("identity").getString("peer");JSONObject pins=new HostStore(t.getTargetContext()).all();if(pins.length()!=2)throw new Exception("Expected both saved hosts");
  if(AccessService.enabled)t.click("Stop browser access");t.await(()->!AccessService.enabled,"Access did not stop");t.click("Start browser access");t.await(()->AccessService.enabled,"Access did not start");
  String host=t.args.getString("host_a");Session initial=recover(t,host);websocket(initial);t.stage("websocket");
  Instrumentation.ActivityMonitor monitor=t.addMonitor(MainActivity.class.getName(),null,false);t.runOnMainSync(()->t.activity.recreate());t.activity=(MainActivity)t.waitForMonitorWithTimeout(monitor,15000);t.removeMonitor(monitor);if(t.activity==null||!AccessService.enabled||!initial.request("/").contains("lifecycle-fixture"))throw new Exception("Activity recreation lost access");t.stage("activity-recreated");
  // Keep Chrome foreground while a real SSE response traverses the app gateway.
  t.click("Open Desktop / website");t.await(()->t.browserText("WispShell"),"Lifecycle browser handoff failed");
  ExecutorService reader=Executors.newSingleThreadExecutor();try{
   final Session stream=recover(t,host);Future<Integer> events=reader.submit(()->{try(Socket socket=stream.socket("/stream","GET")){BufferedReader lines=new BufferedReader(new InputStreamReader(socket.getInputStream()));int count=0;String line;while((line=lines.readLine())!=null)if(line.startsWith("data:"))count++;return count;}});
   barrier(t,"screen-off-ready");if(events.get(60,TimeUnit.SECONDS)!=40||!AccessService.enabled)throw new Exception("Screen-off SSE handoff did not complete");t.stage("screen-off-stream");
   barrier(t,"doze-ready");Session afterIdle=recover(t,host);if(!afterIdle.request("/").contains("lifecycle-fixture"))throw new Exception("Access failed after device idle");t.stage("doze-recovered");
   Session interrupted=recover(t,host);Socket upload=interrupted.socket("/hold","POST");BufferedReader lines=new BufferedReader(new InputStreamReader(upload.getInputStream()));String line;boolean accepted=false;while((line=lines.readLine())!=null){if(line.equals("data: accepted")){accepted=true;break;}}if(!accepted)throw new Exception("Side-effecting POST not accepted");
   Future<Boolean> ended=reader.submit(()->{try{while(lines.readLine()!=null){}return true;}catch(IOException lost){return true;}finally{upload.close();}});
   barrier(t,"wifi-drop-ready");if(!ended.get(15,TimeUnit.SECONDS))throw new Exception("Interrupted POST stayed active");closed(interrupted.url);
   Session recovered=recover(t,host);if(!recovered.request("/count").contains("post-count=1"))throw new Exception("Side-effecting request replayed");t.stage("wifi-recovered-no-post-replay");
   barrier(t,"relay-restart-ready");recovered=recover(t,host);websocket(recovered);t.stage("relay-recovered");
   barrier(t,"network-switch-ready");recovered=recover(t,host);if(!peer.equals(Native.command("identity").getString("peer"))||!new HostStore(t.getTargetContext()).all().has(t.args.getString("host_b")))throw new Exception("Recovery changed identity or pairings");t.stage("network-switch-recovered");
   // Invoke the actual notification PendingIntent rather than bypassing service cancellation.
   boolean stopped=false;for(android.service.notification.StatusBarNotification n:t.getTargetContext().getSystemService(NotificationManager.class).getActiveNotifications()){if(n.getId()==1){n.getNotification().actions[0].actionIntent.send();stopped=true;}}
   if(!stopped)throw new Exception("Foreground Stop notification missing");t.await(()->!AccessService.enabled,"Notification stop failed");closed(recovered.url);t.stage("notification-stopped");
   t.resume();t.click("Start browser access");t.await(()->AccessService.enabled,"Access restart failed");Session killed=recover(t,host);
   // Non-secret port/identity evidence lets the harness prove process death closes listeners.
   File evidence=new File(t.getTargetContext().getFilesDir(),"lifecycle-evidence.json");java.nio.file.Files.write(evidence.toPath(),new JSONObject().put("port",killed.url.port).put("peer",peer).put("hosts",pins.length()).toString().getBytes(java.nio.charset.StandardCharsets.UTF_8));
   barrier(t,"kill-ready");
  }finally{reader.shutdownNow();}
 }
}
