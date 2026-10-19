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
 private Messenger discovery;
 private EditText host,service;
 private TextView status;
 private String endpoint,credential;
 private final ExecutorService work=Executors.newSingleThreadExecutor();
 private boolean bound;
 private final Messenger replies=new Messenger(new Handler(Looper.getMainLooper()){
  public void handleMessage(Message message){
   Bundle result=message.getData();String state=result.getString("status");status.setText(state);
   if("permission_required".equals(state)){
    PendingIntent consent=result.getParcelable("consent",PendingIntent.class);
    try{startIntentSenderForResult(consent.getIntentSender(),1,null,0,0,0,ActivityOptions.makeBasic().setPendingIntentBackgroundActivityStartMode((Build.VERSION.SDK_INT>=36?ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOW_IF_VISIBLE:ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED)).toBundle());}catch(Exception failure){status.setText("consent_failed");}
   }else if("ok".equals(state)){
    endpoint=result.getString("url");credential=result.getString("access_token");fetch();
   }
  }
 });
 private final ServiceConnection connection=new ServiceConnection(){
  public void onServiceConnected(ComponentName name,IBinder binder){discovery=new Messenger(binder);status.setText("connected");if(getIntent().getBooleanExtra("resolve",false))resolve();}
  public void onServiceDisconnected(ComponentName name){discovery=null;status.setText("disconnected");}
 };
 public void onCreate(Bundle state){
  super.onCreate(state);LinearLayout layout=new LinearLayout(this);layout.setOrientation(1);layout.setPadding(24,48,24,24);setContentView(layout);
  host=new EditText(this);host.setHint("Full host ID");host.setText(getIntent().getStringExtra("host_id"));layout.addView(host);
  service=new EditText(this);service.setHint("Service ID");service.setText(getIntent().getStringExtra("service_id"));layout.addView(service);
  status=new TextView(this);status.setContentDescription("discovery_status");layout.addView(status);
  Button open=new Button(this);open.setText("Resolve service");open.setOnClickListener(v->resolve());layout.addView(open);
  Button again=new Button(this);again.setText("Call saved endpoint");again.setOnClickListener(v->fetch());layout.addView(again);
  String target=getIntent().getStringExtra("wispnet_package");if(target==null)target="app.wispnet.android";
  bound=bindService(new Intent().setComponent(new ComponentName(target,"com.biplabs.wispnet.ServiceDiscoveryService")),connection,BIND_AUTO_CREATE);
  if(!bound)status.setText("Wispnet is not installed");
 }
 private void resolve(){
  if(discovery==null){status.setText("disconnected");return;}
  Bundle request=new Bundle();request.putInt("api_version",1);request.putString("request_id","example");request.putString("package_name",getIntent().getStringExtra("claim_package") == null?getPackageName():getIntent().getStringExtra("claim_package"));
  request.putString("host_id",host.getText().toString());request.putString("service_id",service.getText().toString());
  Message message=Message.obtain(null,1);message.setData(request);message.replyTo=replies;
  try{discovery.send(message);}catch(RemoteException failure){status.setText("disconnected");}
 }
 protected void onActivityResult(int request,int result,Intent data){super.onActivityResult(request,result,data);if(request==1){status.setText(result==RESULT_OK?"approved; resolve again when Wispnet is online":"denied");if(result==RESULT_OK)new Handler(getMainLooper()).postDelayed(this::resolve,1200);}}
 private void fetch(){
  if(endpoint==null)return;
  work.execute(()->{
   try{
    URI uri=URI.create(endpoint);String path=getIntent().getStringExtra("path");if(path==null)path="/health";
    // Java clients can connect to the loopback IP and send the returned authority.
    // No dependency on platform wildcard .localhost DNS support.
    try(Socket socket=new Socket()){
     socket.connect(new InetSocketAddress("127.0.0.1",uri.getPort()),5000);socket.setSoTimeout(15000);
     String request="GET "+path+" HTTP/1.1\r\nHost: "+uri.getRawAuthority()+"\r\nX-Wispnet-Access: "+credential+"\r\nConnection: close\r\n\r\n";
     socket.getOutputStream().write(request.getBytes(StandardCharsets.US_ASCII));
     String line=new BufferedReader(new InputStreamReader(socket.getInputStream(),StandardCharsets.UTF_8)).readLine();
     runOnUiThread(()->status.setText(line==null?"connection_closed":line));
    }
   }catch(Exception failure){runOnUiThread(()->status.setText("endpoint_unavailable"));}
  });
 }
 protected void onDestroy(){if(bound)unbindService(connection);work.shutdownNow();super.onDestroy();}
}
