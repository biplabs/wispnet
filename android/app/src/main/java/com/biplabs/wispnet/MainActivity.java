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

public class MainActivity extends Activity {
 boolean managesHosts(){return false;}
 void manageHosts(){startActivity(new Intent(this,HostManagementActivity.class));}
 final ExecutorService work=Executors.newFixedThreadPool(4);
 final java.util.ArrayList<Runnable> ageUpdates=new java.util.ArrayList<>();
 LinearLayout root,list;TextView status,identity,network;
 final Handler lifecycle=new Handler(Looper.getMainLooper());
 final Runnable updateAccess=new Runnable(){public void run(){syncSpace();for(Runnable age:ageUpdates)age.run();lifecycle.postDelayed(this,200);}};EditText label,host,invitation;Button access;
 LinearLayout pairingForm,header,machinesList;FrameLayout space,screen;ScrollView pairingScroll,servicesScroll;StarJump stars;TextView caption;boolean spaceShown=false,enterRequested=false,jumping=false,starting=false;long startAt;String lastNetwork="";
 static volatile long browserExpiresAt=0;
 CheckBox hostVerified;
 HostStore store;
 boolean pairingBusy=false;
 final java.util.Set<String> refreshing=ConcurrentHashMap.newKeySet();
 void text(LinearLayout parent,String value,int size){TextView t=new TextView(this);t.setText(value);t.setTextColor(0xffa5b2c9);t.setTextSize(size);t.setPadding(0,12,0,8);parent.addView(t);}
 Button button(LinearLayout parent,String title,Runnable action){Button b=new Button(this);b.setText(title);b.setTextColor(0xffe9eef8);b.setAllCaps(false);b.setBackground(tint(0xff141c2c,18));parent.addView(b);b.setOnClickListener(v->action.run());return b;}
 EditText field(String hint,int type){EditText e=new EditText(this);e.setHint(hint);e.setInputType(type);pairingForm.addView(e);return e;}
 public void onCreate(Bundle state){
  super.onCreate(state);getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);store=new HostStore(this);
  getWindow().getAttributes().layoutInDisplayCutoutMode=WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS;
  getWindow().setDecorFitsSystemWindows(false);
  root=new LinearLayout(this);root.setOrientation(LinearLayout.VERTICAL);screen=new FrameLayout(this);screen.setBackgroundColor(0xff070b14);stars=new StarJump(this);screen.addView(stars,new FrameLayout.LayoutParams(-1,-1));screen.addView(root,new FrameLayout.LayoutParams(-1,-1));setContentView(screen);immersive();
  getWindow().setDecorFitsSystemWindows(false);root.setOnApplyWindowInsetsListener((view,insets)->{android.graphics.Insets bars=insets.getInsets(WindowInsets.Type.displayCutout()|WindowInsets.Type.ime());view.setPadding(dp(20)+bars.left,bars.top,dp(20)+bars.right,bars.bottom);return insets;});
  header=new LinearLayout(this);header.setGravity(Gravity.CENTER_VERTICAL|Gravity.RIGHT);root.addView(header,new LinearLayout.LayoutParams(-1,dp(64)));

  Button machines=button(header,managesHosts()?"‹":"⚙",()->{if(managesHosts())finish();else manageHosts();});machines.setContentDescription(managesHosts()?"Back to space":"Manage hosts");machines.setLayoutParams(new LinearLayout.LayoutParams(dp(48),dp(48)));
  status=new TextView(this);status.setTextColor(0xffa5c7ff);status.setTextSize(14);status.setPadding(0,dp(8),0,dp(12));status.setVisibility(View.GONE);root.addView(status);
  network=new TextView(this);identity=new TextView(this);identity.setTextColor(0xff7e8aa4);identity.setTextSize(12);identity.setTextIsSelectable(true);
  space=new FrameLayout(this);root.addView(space,new LinearLayout.LayoutParams(-1,0,1));
  servicesScroll=new ScrollView(this);list=new LinearLayout(this);list.setOrientation(LinearLayout.VERTICAL);list.setPadding(0,dp(72),0,dp(24));servicesScroll.addView(list);space.addView(servicesScroll,new FrameLayout.LayoutParams(-1,-1));servicesScroll.setVisibility(View.GONE);
  access=new SpaceToggle(this);access.setAllCaps(false);access.setTextColor(0xffe8edff);access.setTextSize(18);access.setOnClickListener(v->toggleSpace());space.addView(access);
  caption=new TextView(this);caption.setText("enter wisp space");caption.setTextSize(22);caption.setTextColor(0xffe8edff);caption.setGravity(Gravity.CENTER);FrameLayout.LayoutParams cap=new FrameLayout.LayoutParams(-1,dp(48),Gravity.CENTER);cap.topMargin=dp(56);space.addView(caption,cap);
  pairingScroll=new ScrollView(this);pairingForm=new LinearLayout(this);pairingForm.setOrientation(LinearLayout.VERTICAL);pairingScroll.addView(pairingForm);root.addView(pairingScroll,new LinearLayout.LayoutParams(-1,0,1));
  machinesList=new LinearLayout(this);machinesList.setOrientation(LinearLayout.VERTICAL);pairingForm.addView(machinesList);
  text(pairingForm,"Pair a machine",26);button(pairingForm,"Scan pairing QR",this::scanPairingQr);
  button(pairingForm,"Import QR image",()->{Intent pick=new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("image/*").addCategory(Intent.CATEGORY_OPENABLE);startActivityForResult(pick,3);});
  text(pairingForm,"Or use a pairing code",15);label=field("Machine name",InputType.TYPE_CLASS_TEXT);host=field("Host ID",InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);invitation=field("Pairing code",InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_VARIATION_PASSWORD|InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
  button(pairingForm,"Review pasted invitation",()->reviewInvitation(invitation.getText().toString()));hostVerified=new CheckBox(this);hostVerified.setTextColor(0xffa5b2c9);hostVerified.setText("I compared this host ID with the target computer");pairingForm.addView(hostVerified);
  host.addTextChangedListener(new android.text.TextWatcher(){public void beforeTextChanged(CharSequence s,int start,int count,int after){} public void onTextChanged(CharSequence s,int start,int before,int count){hostVerified.setChecked(false);} public void afterTextChanged(android.text.Editable s){}});
  button(pairingForm,"Import invitation file",()->{Intent pick=new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("*/*").addCategory(Intent.CATEGORY_OPENABLE);startActivityForResult(pick,1);});button(pairingForm,"Request pairing",this::pair);
  button(pairingForm,"Refresh all hosts",this::refresh);pairingForm.addView(identity);
  boolean paired=false;try{paired=store.all().length()>0;}catch(Exception ignored){}showPairing(managesHosts());if(!paired&&!managesHosts()){caption.setText("pair your first machine");access.setContentDescription("Pair your first machine");access.setOnClickListener(v->manageHosts());}dockToggle(AccessService.enabled);spaceShown=AccessService.enabled;servicesScroll.setVisibility(spaceShown?View.VISIBLE:View.GONE);
  if(checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)!=android.content.pm.PackageManager.PERMISSION_GRANTED)requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS},2);
  status.setText("");
  receiveInvitation(getIntent());
  work.execute(()->{try{Native.ensure(this);String peer=Native.command("identity").getString("peer");runOnUiThread(()->{identity.setText("Device ID\n"+peer);if(status.getText().toString().startsWith("Preparing protected"))status.setText("Ready. Pair with each machine independently.");refresh();});}catch(Exception e){show("Cannot initialize protected identity: "+e.getMessage());}});
 }
 int dp(int value){return Math.round(value*getResources().getDisplayMetrics().density);}
 android.graphics.drawable.GradientDrawable tint(int color,int radius){android.graphics.drawable.GradientDrawable d=new android.graphics.drawable.GradientDrawable();d.setColor(color);d.setCornerRadius(dp(radius));return d;}
 void showPairing(boolean visible){pairingScroll.setVisibility(visible?View.VISIBLE:View.GONE);space.setVisibility(visible?View.GONE:View.VISIBLE);}
 void dockToggle(boolean docked){
  float fromX=access.getX(),fromY=access.getY();boolean move=docked&&!access.isSelected()&&access.isLaidOut()&&android.animation.ValueAnimator.areAnimatorsEnabled();
  FrameLayout.LayoutParams p=new FrameLayout.LayoutParams(dp(docked?76:100),dp(56),docked?(Gravity.TOP|Gravity.RIGHT):Gravity.CENTER);p.topMargin=docked?dp(4):0;access.setLayoutParams(p);access.setText("");access.setContentDescription(docked?"Leave wisp space":"Enter wisp space");access.setAlpha(1);access.setSelected(docked);access.setTag(docked?"Stop browser access":"Start browser access");caption.setVisibility(docked?View.GONE:View.VISIBLE);if(move)access.post(()->{access.setTranslationX(fromX-access.getLeft());access.setTranslationY(fromY-access.getTop());access.animate().translationX(0).translationY(0).setDuration(450);});
 }
 void toggleSpace(){
  if(starting||jumping)return;
  if(AccessService.enabled){stopService(new Intent(this,AccessService.class));return;}
  try{starting=true;enterRequested=true;startAt=SystemClock.elapsedRealtime();access.setEnabled(false);access.setText("…");startForegroundService(new Intent(this,AccessService.class));}catch(RuntimeException e){starting=false;enterRequested=false;access.setEnabled(true);show("Android could not start access. Try again.");}
 }
 void syncSpace(){
  if(access==null)return;boolean active=AccessService.enabled;
  if(active&&enterRequested&&!jumping){starting=false;enterRequested=false;jumping=true;status.setVisibility(View.GONE);caption.setVisibility(View.GONE);root.animate().alpha(0).setDuration(180);access.setEnabled(false);stars.jump(()->{jumping=false;root.animate().alpha(1).setDuration(300);if(AccessService.enabled){spaceShown=true;dockToggle(true);servicesScroll.setVisibility(View.VISIBLE);servicesScroll.setAlpha(0);servicesScroll.animate().alpha(1).setDuration(350);refresh();}access.setEnabled(true);});}
  else if(active&&!jumping&&!spaceShown){spaceShown=true;dockToggle(true);servicesScroll.setVisibility(View.VISIBLE);}
  else if(!active&&(spaceShown||jumping)){stars.cancel();root.animate().cancel();root.setAlpha(1);jumping=false;spaceShown=false;servicesScroll.setVisibility(View.GONE);dockToggle(false);access.setEnabled(true);}
  if(starting&&SystemClock.elapsedRealtime()-startAt>15000){starting=false;enterRequested=false;dockToggle(false);access.setEnabled(true);stopService(new Intent(this,AccessService.class));show("Access could not start. Try again.");}
  String current=AccessService.networkStatus;network.setText(current);if(active&&!current.equals(lastNetwork)&&!current.equals("Network available"))show(current);lastNetwork=current;
 }
 void show(String value){runOnUiThread(()->{status.setText(value);status.setVisibility(View.VISIBLE);status.announceForAccessibility(value);if(pairingScroll.getVisibility()==View.VISIBLE)pairingScroll.post(()->pairingScroll.smoothScrollTo(0,0));});}
 void pair(){final String expected=host.getText().toString().trim(),document=invitation.getText().toString(),name=label.getText().toString().trim();
  if(!hostVerified.isChecked()){show("Compare the full host ID with the target computer, then confirm it before requesting pairing.");return;}
  if(expected.isEmpty()||document.isEmpty()||name.isEmpty()){show("Enter machine name, verified host ID and invitation");return;}
  requestPairing(expected,document,name);
 }
 void requestPairing(String expected,String document,String name){
  if(pairingBusy){show("A pairing request is already in progress. Please wait.");return;}
  pairingBusy=true;invitation.setText("");show("Requesting pairing from "+name+"… This may take up to a minute.");
  work.execute(()->{try{Native.ensure(this);Native.request(new JSONObject().put("op","pair").put("host",expected).put("invitation",document));store.paired(expected,name);show("Pending owner approval · "+name);runOnUiThread(()->{if(managesHosts())finish();else{showPairing(false);render();}});}catch(Exception e){if(e instanceof Native.Failure&&"invitation_denied".equals(((Native.Failure)e).code))show("This host did not accept the invitation. Ask its owner for a new invitation.");else show("Pairing failed: "+e.getMessage()+". Scan a fresh invitation to try again.");}finally{runOnUiThread(()->pairingBusy=false);}});
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
  list.removeAllViews();ageUpdates.clear();machinesList.removeAllViews();
  LinearLayout heading=new LinearLayout(this);heading.setGravity(Gravity.CENTER_VERTICAL|Gravity.RIGHT);list.addView(heading);Button reload=button(heading,"↻",this::refresh);reload.setContentDescription("Refresh services");reload.setTag("Refresh all hosts");reload.setLayoutParams(new LinearLayout.LayoutParams(dp(48),dp(48)));
  try{JSONObject records=store.all();if(!managesHosts()&&!spaceShown&&!starting&&!jumping)caption.setText(records.length()>0?"enter wisp space":"pair your first machine");if(records.length()==0){text(list,"Pair a machine to get started",16);return;}
   for(java.util.Iterator<String> keys=records.keys();keys.hasNext();){String peer=keys.next();JSONObject record=records.getJSONObject(peer);String name=record.getString("name"),state=record.optString("state");JSONObject catalog=record.optJSONObject("catalog");
    Button machine=button(machinesList,name+"\n"+record.optString("status"),()->new AlertDialog.Builder(this).setTitle(name).setMessage("Host ID\n"+peer+"\n\n"+record.optString("status")).setPositiveButton("Refresh",(dialog,which)->refresh()).setNegativeButton("Done",null).show());machine.setPadding(dp(16),dp(12),dp(16),dp(12));
    if(catalog==null){text(list,name+" · "+("approval_pending".equals(state)?"Awaiting approval":"Unavailable"),16);continue;}
    JSONArray services=catalog.getJSONArray("services");if(services.length()==0)text(list,name+" · No shared services",15);
    for(int i=0;i<services.length();i++){String service=services.getJSONObject(i).getString("id");String display=service.equals("llama")?"Llama":service.equals("website")?"Website":service;
     Button row=button(list,display+"   ↗\n"+name,()->open(peer,service));row.setTag("Open "+name+" / "+service);row.setGravity(Gravity.LEFT|Gravity.CENTER_VERTICAL);row.setTextSize(18);row.setPadding(dp(22),dp(16),dp(22),dp(16));LinearLayout.LayoutParams params=new LinearLayout.LayoutParams(-1,dp(92));params.bottomMargin=dp(12);row.setLayoutParams(params);
     boolean allowed=!java.util.List.of("host_denied","approval_pending","state_changed","unverified").contains(state);row.setEnabled(allowed);row.setAlpha(allowed?1:.45f);if(!"available".equals(state))row.setText(display+"   ↗\n"+name+" · "+("host_denied".equals(state)?"Denied":"approval_pending".equals(state)?"Pending":"unreachable".equals(state)?"Offline · cached":"Unavailable"));row.setContentDescription(display+", "+name+", "+record.optString("status"));
     row.setOnLongClickListener(v->{new AlertDialog.Builder(this).setTitle(name).setMessage(record.optString("status")+"\n\nHost ID\n"+peer+"\n\nDirectory age: "+Math.max(0,(System.currentTimeMillis()-record.optLong("fetched"))/1000)+" seconds").setPositiveButton("Done",null).show();return true;});
    }
   }
  }catch(Exception e){show("Saved directory unavailable");}
 }
 void open(String peer,String service){if(!AccessService.enabled){show("Start browser access first");return;}show("Opening "+service+"…");work.execute(()->{try{JSONObject response=Native.request(new JSONObject().put("op","open").put("host",peer).put("service",service));String url=response.getString("url");runOnUiThread(()->{browserExpiresAt=SystemClock.elapsedRealtime()+response.optLong("expires_in",600)*1000;status.setVisibility(View.GONE);Intent browser=new Intent(Intent.ACTION_VIEW,Uri.parse(url));browser.addCategory(Intent.CATEGORY_BROWSABLE);try{startActivity(browser);}catch(ActivityNotFoundException missing){show("Install a browser to open this service.");}});}catch(Exception e){try{store.failed(peer,e);}catch(Exception ignored){}show("Cannot open service: "+e.getMessage());runOnUiThread(this::render);}});}
 void scanPairingQr(){
  if(!getPackageManager().hasSystemFeature(android.content.pm.PackageManager.FEATURE_CAMERA_ANY)){show("No camera available. Import a QR image or invitation file, or paste a pairing code.");return;}
  try{com.journeyapps.barcodescanner.ScanOptions options=new com.journeyapps.barcodescanner.ScanOptions().setCaptureActivity(PairingCaptureActivity.class).setDesiredBarcodeFormats(com.journeyapps.barcodescanner.ScanOptions.QR_CODE).setBeepEnabled(false).setBarcodeImageEnabled(false).setTimeout(60000).setPrompt("Scan your computer’s Wispnet QR to pair.");startActivityForResult(options.createScanIntent(this),2);}catch(RuntimeException unavailable){show("Camera unavailable. Import a QR image or invitation file, or paste a pairing code.");}
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
 void immersive(){WindowInsetsController controller=getWindow().getInsetsController();if(controller!=null){controller.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);controller.hide(WindowInsets.Type.systemBars());}}
 public void onWindowFocusChanged(boolean focused){super.onWindowFocusChanged(focused);if(focused)immersive();}
 protected void onResume(){super.onResume();immersive();if(store!=null&&!managesHosts())try{boolean paired=store.all().length()>0;access.setOnClickListener(v->{try{if(store.all().length()>0)toggleSpace();else manageHosts();}catch(Exception e){show("Cannot read saved hosts");}});caption.setText(paired?"enter wisp space":"pair your first machine");}catch(Exception ignored){}if(AccessService.enabled&&browserExpiresAt>0&&SystemClock.elapsedRealtime()>=browserExpiresAt)status.setText("Browser session expired. Open the service again; pairing is unchanged.");lifecycle.post(updateAccess);syncSpace();}
 protected void onPause(){lifecycle.removeCallbacks(updateAccess);super.onPause();}
 protected void onDestroy(){root.animate().cancel();stars.cancel();access.animate().cancel();work.shutdown();super.onDestroy();}
}
