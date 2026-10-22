package app.wispnet.discoveryexample;

import android.app.*;
import android.content.*;
import android.os.*;
import android.widget.*;
import java.net.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.*;

/** Independent UID: demonstrates discovery without sharing Wispnet's code or data. */
public final class MainActivity extends Activity {
 private EditText host,service,registry;
 private TextView status;
 private String endpoint;
 private final ExecutorService work=Executors.newSingleThreadExecutor();
 public void onCreate(Bundle state){
  super.onCreate(state);LinearLayout layout=new LinearLayout(this);layout.setOrientation(1);layout.setPadding(24,48,24,24);setContentView(layout);
  host=new EditText(this);host.setHint("Full host ID");host.setText(getIntent().getStringExtra("host_id"));layout.addView(host);
  service=new EditText(this);service.setHint("Service ID");service.setText(getIntent().getStringExtra("service_id"));layout.addView(service);
  status=new TextView(this);status.setContentDescription("discovery_status");layout.addView(status);
  Button open=new Button(this);open.setText("Resolve service");open.setOnClickListener(v->resolve());layout.addView(open);
  Button again=new Button(this);again.setText("Call saved endpoint");again.setOnClickListener(v->fetch());layout.addView(again);
  registry=new EditText(this);registry.setHint("Registry URL");registry.setText("http://127.0.0.1:"+getIntent().getIntExtra("registry_port",18400));layout.addView(registry);
  if(getIntent().getBooleanExtra("resolve",false))resolve();
 }
 private void resolve(){
  final String base=registry.getText().toString(),hostId=host.getText().toString(),serviceId=service.getText().toString();status.setText("resolving");
  work.execute(()->{try{
   HttpURLConnection connection=(HttpURLConnection)new URL(base+"/v1/resolve").openConnection();connection.setConnectTimeout(5000);connection.setReadTimeout(25000);connection.setRequestMethod("POST");connection.setDoOutput(true);connection.setRequestProperty("Content-Type","application/json");
   byte[] body=new org.json.JSONObject().put("host_id",hostId).put("service_id",serviceId).toString().getBytes(StandardCharsets.UTF_8);connection.setFixedLengthStreamingMode(body.length);try(OutputStream stream=connection.getOutputStream()){stream.write(body);}
   try{if(connection.getResponseCode()!=200){runOnUiThread(()->status.setText("service_unavailable"));return;}org.json.JSONObject result=new org.json.JSONObject(new String(connection.getInputStream().readAllBytes(),StandardCharsets.UTF_8));endpoint=result.getString("url");}finally{connection.disconnect();}
   fetch();
  }catch(Exception failure){runOnUiThread(()->status.setText("service_unavailable"));}});
 }
 private void fetch(){
  if(endpoint==null)return;
  work.execute(()->{
   try{
    URI uri=URI.create(endpoint);String path=getIntent().getStringExtra("path");if(path==null)path="/health";
    // Java clients can connect to the loopback IP and send the returned authority.
    // No dependency on platform wildcard .localhost DNS support.
    try(Socket socket=new Socket()){
     socket.connect(new InetSocketAddress("127.0.0.1",uri.getPort()),5000);socket.setSoTimeout(15000);
     String request="GET "+path+" HTTP/1.1\r\nHost: "+uri.getRawAuthority()+"\r\nConnection: close\r\n\r\n";
     socket.getOutputStream().write(request.getBytes(StandardCharsets.US_ASCII));
     String line=new BufferedReader(new InputStreamReader(socket.getInputStream(),StandardCharsets.UTF_8)).readLine();
     runOnUiThread(()->status.setText(line==null?"connection_closed":line));
    }
   }catch(Exception failure){runOnUiThread(()->status.setText("endpoint_unavailable"));}
  });
 }
 protected void onDestroy(){work.shutdownNow();super.onDestroy();}
}
