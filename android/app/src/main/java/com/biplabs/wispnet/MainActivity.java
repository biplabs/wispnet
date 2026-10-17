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
 CheckBox hostVerified;
 HostStore store;
 boolean pairingBusy=false;
 final java.util.Set<String> refreshing=ConcurrentHashMap.newKeySet();
 void text(LinearLayout parent,String value,int size){TextView t=new TextView(this);t.setText(value);t.setTextSize(size);t.setPadding(0,12,0,8);parent.addView(t);}
 Button button(LinearLayout parent,String title,Runnable action){Button b=new Button(this);b.setText(title);parent.addView(b);b.setOnClickListener(v->action.run());return b;}
 EditText field(String hint,int type){EditText e=new EditText(this);e.setHint(hint);e.setInputType(type);root.addView(e);return e;}
 public void onCreate(Bundle state){
  super.onCreate(state);getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);store=new HostStore(this);
  ScrollView scroll=new ScrollView(this);root=new LinearLayout(this);root.setOrientation(LinearLayout.VERTICAL);root.setPadding(24,24,24,32);scroll.addView(root);setContentView(scroll);
  getWindow().setDecorFitsSystemWindows(false);scroll.setOnApplyWindowInsetsListener((view,insets)->{android.graphics.Insets bars=insets.getInsets(WindowInsets.Type.systemBars()|WindowInsets.Type.displayCutout()|WindowInsets.Type.ime());view.setPadding(bars.left,bars.top,bars.right,bars.bottom);return insets;});
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
  invitation=field("Paste pairing code or invitation JSON",InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_VARIATION_PASSWORD|InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
  text(root,"No camera needed: paste a pairing code, import a file, or share an invitation to Wispnet.",15);
  button(root,"Scan pairing QR",this::scanPairingQr);
  button(root,"Import QR image",()->{Intent pick=new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("image/*").addCategory(Intent.CATEGORY_OPENABLE);startActivityForResult(pick,3);});
  button(root,"Review pasted invitation",()->reviewInvitation(invitation.getText().toString()));
  hostVerified=new CheckBox(this);hostVerified.setText("I compared this host ID with the target computer");root.addView(hostVerified);
  host.addTextChangedListener(new android.text.TextWatcher(){public void beforeTextChanged(CharSequence s,int start,int count,int after){} public void onTextChanged(CharSequence s,int start,int before,int count){hostVerified.setChecked(false);} public void afterTextChanged(android.text.Editable s){}});
  button(root,"Import invitation file",()->{Intent pick=new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("*/*").addCategory(Intent.CATEGORY_OPENABLE);startActivityForResult(pick,1);});
  button(root,"Request pairing",this::pair);
  text(root,"Services",22);list=new LinearLayout(this);list.setOrientation(LinearLayout.VERTICAL);root.addView(list);
  if(checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)!=android.content.pm.PackageManager.PERMISSION_GRANTED)requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS},2);
  status.setText("Preparing protected device identity…");
  receiveInvitation(getIntent());
  work.execute(()->{try{Native.ensure(this);String peer=Native.command("identity").getString("peer");runOnUiThread(()->{identity.setText("This tablet: "+peer);if(status.getText().toString().startsWith("Preparing protected"))status.setText("Ready. Pair with each machine independently.");refresh();});}catch(Exception e){show("Cannot initialize protected identity: "+e.getMessage());}});
 }
 void show(String value){runOnUiThread(()->{status.setText(value);status.announceForAccessibility(value);if(root.getParent() instanceof ScrollView){ScrollView scroll=(ScrollView)root.getParent();scroll.post(()->scroll.smoothScrollTo(0,0));}});}
 void pair(){final String expected=host.getText().toString().trim(),document=invitation.getText().toString(),name=label.getText().toString().trim();
  if(!hostVerified.isChecked()){show("Compare the full host ID with the target computer, then confirm it before requesting pairing.");return;}
  if(expected.isEmpty()||document.isEmpty()||name.isEmpty()){show("Enter machine name, verified host ID and invitation");return;}
  requestPairing(expected,document,name);
 }
 void requestPairing(String expected,String document,String name){
  if(pairingBusy){show("A pairing request is already in progress. Please wait.");return;}
  pairingBusy=true;invitation.setText("");show("Requesting pairing from "+name+"… This may take up to a minute.");
  work.execute(()->{try{Native.ensure(this);Native.request(new JSONObject().put("op","pair").put("host",expected).put("invitation",document));store.paired(expected,name);show("Request sent. Pending owner approval on "+name+". Grant services on that computer, then refresh.");runOnUiThread(this::render);}catch(Exception e){if(e instanceof Native.Failure&&"invitation_denied".equals(((Native.Failure)e).code))show("This host did not accept the invitation. Ask its owner for a new invitation.");else show("Pairing failed: "+e.getMessage()+". Scan a fresh invitation to try again.");}finally{runOnUiThread(()->pairingBusy=false);}});
 }
 void pairScannedQr(String document){
  if(pairingBusy){show("A pairing request is already in progress. Please wait.");return;}
  if(document.getBytes(java.nio.charset.StandardCharsets.UTF_8).length>8192){show("Invitation is too large. Ask for a new QR code.");return;}
  invitation.setText("");hostVerified.setChecked(false);show("Checking scanned invitation…");
  work.execute(()->{try{Native.ensure(this);JSONObject preview=Native.request(new JSONObject().put("op","preview_invitation").put("invitation",document));String target=preview.getString("host");runOnUiThread(()->{host.setText(target);String name=preview.optString("host_name","").trim();if(name.isEmpty())name="Host "+target.substring(0,8);label.setText(name);requestPairing(target,document,name);});}catch(Exception e){show("Cannot import invitation: "+e.getMessage());}});
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
 void scanPairingQr(){
  if(!getPackageManager().hasSystemFeature(android.content.pm.PackageManager.FEATURE_CAMERA_ANY)){show("No camera available. Import a QR image or invitation file, or paste a pairing code.");return;}
  try{com.journeyapps.barcodescanner.ScanOptions options=new com.journeyapps.barcodescanner.ScanOptions().setCaptureActivity(PairingCaptureActivity.class).setDesiredBarcodeFormats(com.journeyapps.barcodescanner.ScanOptions.QR_CODE).setBeepEnabled(false).setBarcodeImageEnabled(false).setTimeout(60000).setPrompt("Scan a Wispnet invitation. You will review the host ID next.");startActivityForResult(options.createScanIntent(this),2);}catch(RuntimeException unavailable){show("Camera unavailable. Import a QR image or invitation file, or paste a pairing code.");}
 }
 void readQrImage(Uri uri){
  invitation.setText("");hostVerified.setChecked(false);status.setText("Reading QR image…");
  work.execute(()->{try{if(uri==null||!"content".equals(uri.getScheme()))throw new IOException();try(InputStream input=getContentResolver().openInputStream(uri)){String document=PairingQr.decode(input);runOnUiThread(()->reviewInvitation(document));}}catch(Exception invalid){show("Cannot read pairing QR image. Import an invitation file or paste its pairing code instead.");}});
 }
 void reviewInvitation(String document){
  if(document.getBytes(java.nio.charset.StandardCharsets.UTF_8).length>8192){invitation.setText("");show("Invitation is too large. Ask for a new invitation file.");return;}
  hostVerified.setChecked(false);invitation.setText("");show("Reviewing invitation…");
  work.execute(()->{try{Native.ensure(this);JSONObject preview=Native.request(new JSONObject().put("op","preview_invitation").put("invitation",document));runOnUiThread(()->{host.setText(preview.optString("host"));String machine=preview.optString("host_name","").trim();if(!machine.isEmpty())label.setText(machine);invitation.setText(document);show("Invitation ready for review. Compare the full host ID on the target computer. Importing does not request pairing or grant access.");});}catch(Exception e){show("Cannot import invitation: "+e.getMessage());}});
 }
 void importInvitation(InputStream in)throws IOException{byte[] bytes=in.readNBytes(8193);if(bytes.length>8192)throw new IOException();String document=new String(bytes,java.nio.charset.StandardCharsets.UTF_8);runOnUiThread(()->reviewInvitation(document));}
 void readInvitation(Uri uri){
  invitation.setText("");hostVerified.setChecked(false);status.setText("Reading invitation file…");
  work.execute(()->{try{if(uri==null||!"content".equals(uri.getScheme()))throw new IOException();try(InputStream in=getContentResolver().openInputStream(uri)){importInvitation(in);}}catch(Exception e){show("Cannot read bounded invitation file. Paste its pairing code instead.");}});
 }
 void receiveInvitation(Intent intent){
  if(!Intent.ACTION_SEND.equals(intent.getAction()))return;
  try{Uri uri=intent.getParcelableExtra(Intent.EXTRA_STREAM,Uri.class);if(uri!=null){readInvitation(uri);}
   else{String document=intent.getStringExtra(Intent.EXTRA_TEXT);if(document==null)throw new IOException();reviewInvitation(document);}
  }catch(Exception invalid){show("Cannot read invitation. Paste a pairing code or import its file instead.");}
  intent.removeExtra(Intent.EXTRA_STREAM);intent.removeExtra(Intent.EXTRA_TEXT);
 }
 protected void onNewIntent(Intent intent){super.onNewIntent(intent);setIntent(intent);receiveInvitation(intent);}
 protected void onActivityResult(int request,int result,Intent data){super.onActivityResult(request,result,data);if(request==1&&result==RESULT_OK&&data!=null)readInvitation(data.getData());else if(request==3&&result==RESULT_OK&&data!=null)readQrImage(data.getData());else if(request==2){if(result==RESULT_OK&&data!=null){String document=data.getStringExtra("SCAN_RESULT");if(document!=null)pairScannedQr(document);else show("No pairing QR found. Import a file or paste a pairing code.");}else show("Scan cancelled or camera unavailable. Import a QR image or invitation file, or paste a pairing code.");}}
 protected void onResume(){super.onResume();if(AccessService.enabled&&browserExpiresAt>0&&SystemClock.elapsedRealtime()>=browserExpiresAt)status.setText("Browser session expired. Open the service again; pairing is unchanged.");lifecycle.post(updateAccess);if(access!=null)access.setText(AccessService.enabled?"Stop browser access":"Start browser access");}
 protected void onPause(){lifecycle.removeCallbacks(updateAccess);super.onPause();}
 protected void onDestroy(){work.shutdown();super.onDestroy();}
}
