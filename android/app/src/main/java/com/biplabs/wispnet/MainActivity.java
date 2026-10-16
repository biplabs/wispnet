package com.biplabs.wispnet;
import android.Manifest;
import android.app.*;
import android.content.*;
import android.net.Uri;
import android.os.*;
import android.text.InputType;
import android.view.*;
import android.widget.*;
import org.json.*;
import java.io.*;
import java.util.concurrent.*;

public final class MainActivity extends Activity {
 final ExecutorService work=Executors.newFixedThreadPool(4);
 final java.util.ArrayList<Runnable> ageUpdates=new java.util.ArrayList<>();
 LinearLayout root,list;TextView status,identity,network;
 final Handler lifecycle=new Handler(Looper.getMainLooper());
 final Runnable updateAccess=new Runnable(){public void run(){if(access!=null){access.setText(AccessService.enabled?"Stop browser access":"Start browser access");network.setText(AccessService.networkStatus);for(Runnable age:ageUpdates)age.run();}lifecycle.postDelayed(this,1000);}};EditText label,host,invitation;Button access;
 static volatile long browserExpiresAt=0;
 HostStore store;
 final java.util.Set<String> refreshing=ConcurrentHashMap.newKeySet();
 void text(LinearLayout parent,String value,int size){TextView t=new TextView(this);t.setText(value);t.setTextSize(size);t.setPadding(0,12,0,8);parent.addView(t);}
 Button button(LinearLayout parent,String title,Runnable action){Button b=new Button(this);b.setText(title);parent.addView(b);b.setOnClickListener(v->action.run());return b;}
 EditText field(String hint,int type){EditText e=new EditText(this);e.setHint(hint);e.setInputType(type);root.addView(e);return e;}
 public void onCreate(Bundle state){
  super.onCreate(state);getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);store=new HostStore(this);
  ScrollView scroll=new ScrollView(this);root=new LinearLayout(this);root.setOrientation(LinearLayout.VERTICAL);root.setPadding(24,24,24,32);scroll.addView(root);setContentView(scroll);
  text(root,"Wispnet",30);text(root,"Your machines. Their services.",18);
  status=new TextView(this);root.addView(status);network=new TextView(this);root.addView(network);identity=new TextView(this);identity.setTextIsSelectable(true);root.addView(identity);
  access=button(root,"Start browser access",()->{
   if(AccessService.enabled){stopService(new Intent(this,AccessService.class));access.setText("Start browser access");status.setText("Browser access stopped");}
   else {try{startForegroundService(new Intent(this,AccessService.class));status.setText("Starting browser access…");}catch(RuntimeException unavailable){status.setText("Android could not start access. Return to Wispnet and try again after the platform restriction ends.");}}
  });
  button(root,"Refresh all hosts",this::refresh);
  text(root,"Pair a machine",22);
  label=field("Machine name",InputType.TYPE_CLASS_TEXT);
  host=field("Host ID — verify on that machine",InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
  invitation=field("Paste invitation JSON",InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_VARIATION_PASSWORD|InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
  button(root,"Import invitation file",()->{Intent pick=new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("*/*").addCategory(Intent.CATEGORY_OPENABLE);startActivityForResult(pick,1);});
  button(root,"Request pairing",this::pair);
  text(root,"Services",22);list=new LinearLayout(this);list.setOrientation(LinearLayout.VERTICAL);root.addView(list);
  if(checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)!=android.content.pm.PackageManager.PERMISSION_GRANTED)requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS},2);
  status.setText("Preparing protected device identity…");
  work.execute(()->{try{Native.ensure(this);String peer=Native.command("identity").getString("peer");runOnUiThread(()->{identity.setText("This tablet: "+peer);if(status.getText().toString().startsWith("Preparing protected"))status.setText("Ready. Pair with each machine independently.");refresh();});}catch(Exception e){show("Cannot initialize protected identity: "+e.getMessage());}});
 }
 void show(String value){runOnUiThread(()->status.setText(value));}
 void pair(){final String expected=host.getText().toString().trim(),document=invitation.getText().toString(),name=label.getText().toString().trim();invitation.setText("");
  if(expected.isEmpty()||document.isEmpty()||name.isEmpty()){show("Enter machine name, verified host ID and invitation");return;}
  status.setText("Requesting pairing from "+name+"…");
  work.execute(()->{try{Native.ensure(this);Native.request(new JSONObject().put("op","pair").put("host",expected).put("invitation",document));store.paired(expected,name);show("Request sent. Approve this tablet on "+name+", grant services, then refresh.");runOnUiThread(this::render);}catch(Exception e){if(e instanceof Native.Failure&&"invitation_denied".equals(((Native.Failure)e).code))show("This host did not accept the invitation. Ask its owner for a new invitation.");else show(e.getMessage());}});
 }
 void refresh(){
  try{JSONObject hosts=store.all();for(java.util.Iterator<String> keys=hosts.keys();keys.hasNext();){String peer=keys.next();if(!refreshing.add(peer))continue;work.execute(()->{try{Native.ensure(this);JSONObject snapshot=Native.request(new JSONObject().put("op","catalog").put("host",peer));store.catalog(peer,snapshot);}catch(Exception e){try{store.failed(peer,e);}catch(Exception ignored){}}refreshing.remove(peer);runOnUiThread(this::render);});}render();}catch(Exception e){show("Cannot read saved hosts");}
 }
 void render(){
  list.removeAllViews();ageUpdates.clear();try{JSONObject records=store.all();if(records.length()==0){text(list,"No paired machines yet",16);return;}
   for(java.util.Iterator<String> keys=records.keys();keys.hasNext();){String peer=keys.next();JSONObject record=records.getJSONObject(peer);
    String name=record.getString("name"),state=record.optString("status","Not refreshed");text(list,name+" · "+peer.substring(0,12),20);text(list,state,15);
    JSONObject catalog=record.optJSONObject("catalog");if(catalog==null)continue;
    long fetched=record.optLong("fetched");TextView age=new TextView(this);age.setTextSize(13);list.addView(age);Runnable updateAge=()->age.setText("Directory fetched "+Math.max(0,(System.currentTimeMillis()-fetched)/1000)+"s ago"+("Available".equals(state)?"":" · cached listing"));ageUpdates.add(updateAge);updateAge.run();
    JSONArray services=catalog.getJSONArray("services");if(services.length()==0)text(list,"No services granted by this host",15);
    for(int i=0;i<services.length();i++){String service=services.getJSONObject(i).getString("id");Button open=button(list,"Open "+name+" / "+service,()->open(peer,service));open.setEnabled(!java.util.List.of("host_denied","approval_pending","state_changed","unverified").contains(record.optString("state")));}
   }
  }catch(Exception e){show("Saved directory unavailable");}
 }
 void open(String peer,String service){if(!AccessService.enabled){show("Start browser access first");return;}status.setText("Opening "+service+"…");work.execute(()->{try{JSONObject response=Native.request(new JSONObject().put("op","open").put("host",peer).put("service",service));String url=response.getString("url");runOnUiThread(()->{browserExpiresAt=SystemClock.elapsedRealtime()+response.optLong("expires_in",600)*1000;status.setText("Browser access active. Reopen this service when its local session expires.");Intent browser=new Intent(Intent.ACTION_VIEW,Uri.parse(url));browser.addCategory(Intent.CATEGORY_BROWSABLE);startActivity(browser);});}catch(Exception e){try{store.failed(peer,e);}catch(Exception ignored){}show("Cannot open service: "+e.getMessage());runOnUiThread(this::render);}});}
 protected void onActivityResult(int request,int result,Intent data){super.onActivityResult(request,result,data);if(request==1&&result==RESULT_OK&&data!=null){try(InputStream in=getContentResolver().openInputStream(data.getData())){byte[] bytes=in.readNBytes(8193);if(bytes.length>8192)throw new IOException();invitation.setText(new String(bytes,java.nio.charset.StandardCharsets.UTF_8));status.setText("Invitation imported. Verify the host ID before pairing.");}catch(Exception e){show("Cannot read bounded invitation file");}}}
 protected void onResume(){super.onResume();if(AccessService.enabled&&browserExpiresAt>0&&SystemClock.elapsedRealtime()>=browserExpiresAt)status.setText("Browser session expired. Open the service again; pairing is unchanged.");lifecycle.post(updateAccess);if(access!=null)access.setText(AccessService.enabled?"Stop browser access":"Start browser access");}
 protected void onPause(){lifecycle.removeCallbacks(updateAccess);super.onPause();}
 protected void onDestroy(){work.shutdown();super.onDestroy();}
}
