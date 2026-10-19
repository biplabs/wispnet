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
 LinearLayout root,list;TextView status,identity,network,deviceHeading,relayLabel;
 int noticeVersion=0;boolean noticeLeaving=false;
 final Runnable dismissNotice=()->dismissNotice();
 long relayCheckedAt=0;boolean relayBusy=false;
 final Handler lifecycle=new Handler(Looper.getMainLooper());
 final Runnable updateAccess=new Runnable(){public void run(){syncSpace();for(Runnable age:ageUpdates)age.run();lifecycle.postDelayed(this,200);}};EditText label,host,invitation;Button access;
 LinearLayout pairingForm,header,machinesList,reviewBox,addMachineBox;FrameLayout space,screen;ScrollView pairingScroll,servicesScroll;StarJump stars;TextView caption,sheetTitle;Button machinesPill,addMachineBtn;boolean spaceShown=false,enterRequested=false,jumping=false,starting=false;long startAt;String lastNetwork="",devicePeer="";
 CheckBox hostVerified;
 HostStore store;Button deviceName;Runnable closeServicePermissions;
 boolean pairingBusy=false;
 boolean openingService=false,serviceJumpDone=false;int serviceLaunch=0;String pendingServiceUrl;
 final java.util.Set<String> refreshing=ConcurrentHashMap.newKeySet();
 final ConcurrentHashMap<String,String> peerTelemetry=new ConcurrentHashMap<>();
 void syncTelemetry(){
  try{JSONObject peers=Native.command("telemetry").optJSONObject("peers");if(peers==null)return;
   for(java.util.Iterator<String> it=peers.keys();it.hasNext();){String k=it.next();JSONObject t=peers.optJSONObject(k);if(t==null)continue;String p=t.optString("path","");long rtt=t.optLong("rtt_ms",0);String label=("direct".equals(p)?"DIRECT":"relay".equals(p)?"RELAY":"");if(rtt>0)label=label.isEmpty()?rtt+"ms":label+" · "+rtt+"ms";if(!label.isEmpty())peerTelemetry.put(k,label);}
  }catch(Exception ignored){}
 }
 void text(LinearLayout parent,String value,int size){TextView t=new TextView(this);t.setText(value);t.setTextColor(size>=20?0xffedf2ff:0xff94a6c4);t.setTextSize(size);t.setTypeface(null,size>=20?android.graphics.Typeface.BOLD:android.graphics.Typeface.NORMAL);t.setPadding(0,dp(size>=20?18:8),0,dp(8));parent.addView(t);}
 Button button(LinearLayout parent,String title,Runnable action){boolean primary=java.util.List.of("Scan with camera","Scan pairing QR","Import QR image","Review pasted invitation","Request pairing").contains(title);Button b=new Button(this);b.setText(title);b.setTextColor(0xffedf2ff);b.setAllCaps(false);b.setTextSize(15);b.setPadding(dp(18),dp(12),dp(18),dp(12));b.setBackground(card(primary?0xff1f3666:0xff142038,primary?0xff4b73c2:0xff283b60,14));LinearLayout.LayoutParams lp=new LinearLayout.LayoutParams(-1,-2);lp.bottomMargin=dp(10);parent.addView(b,lp);b.setOnClickListener(v->action.run());return b;}
 EditText field(LinearLayout parent,String hint,int type){EditText e=new EditText(this);e.setHint(hint);e.setInputType(type);e.setTextColor(0xffedf2ff);e.setHintTextColor(0xff6e82a3);e.setTextSize(15);e.setBackground(card(0xff0d1526,0xff25385c,14));e.setPadding(dp(16),dp(14),dp(16),dp(14));LinearLayout.LayoutParams lp=new LinearLayout.LayoutParams(-1,-2);lp.bottomMargin=dp(10);parent.addView(e,lp);return e;}
 CharSequence styledCardText(String top,String bottom,int bottomColor){android.text.SpannableStringBuilder sb=new android.text.SpannableStringBuilder(top+"\n"+bottom);sb.setSpan(new android.text.style.StyleSpan(android.graphics.Typeface.BOLD),0,top.length(),android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);for(char ch:new char[]{'↗','✦'}){int idx=top.indexOf(ch);if(idx>=0)sb.setSpan(new android.text.style.ForegroundColorSpan(0xff65ceff),idx,idx+1,android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);}int start=top.length()+1,end=sb.length();sb.setSpan(new android.text.style.RelativeSizeSpan(0.78f),start,end,android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);sb.setSpan(new android.text.style.ForegroundColorSpan(bottomColor),start,end,android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);return sb;}
 public void onCreate(Bundle state){
  super.onCreate(state);getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);store=new HostStore(this);
  getWindow().getAttributes().layoutInDisplayCutoutMode=WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS;
  getWindow().setDecorFitsSystemWindows(false);
  root=new LinearLayout(this);root.setOrientation(LinearLayout.VERTICAL);screen=new FrameLayout(this);screen.setBackgroundColor(0xff070b14);stars=new StarJump(this);screen.addView(stars,new FrameLayout.LayoutParams(-1,-1));View statusScrim=new View(this);statusScrim.setBackgroundColor(0x66070b14);screen.addView(statusScrim,new FrameLayout.LayoutParams(-1,0,Gravity.TOP));screen.addView(root,new FrameLayout.LayoutParams(-1,-1));setContentView(screen);configureSystemBars();
  getWindow().setDecorFitsSystemWindows(false);root.setOnApplyWindowInsetsListener((view,insets)->{android.graphics.Insets bars=insets.getInsets(WindowInsets.Type.systemBars()|WindowInsets.Type.displayCutout()|WindowInsets.Type.ime());view.setPadding(dp(20)+bars.left,bars.top,dp(20)+bars.right,bars.bottom);if(status!=null){FrameLayout.LayoutParams notice=(FrameLayout.LayoutParams)status.getLayoutParams();notice.setMargins(dp(20)+bars.left,0,dp(20)+bars.right,dp(20)+bars.bottom);status.setLayoutParams(notice);}int top=insets.getInsets(WindowInsets.Type.statusBars()|WindowInsets.Type.displayCutout()).top;ViewGroup.LayoutParams shade=statusScrim.getLayoutParams();if(shade.height!=top){shade.height=top;statusScrim.setLayoutParams(shade);}return insets;});
  header=new LinearLayout(this);header.setGravity(Gravity.CENTER_VERTICAL|Gravity.LEFT);root.addView(header,new LinearLayout.LayoutParams(-1,dp(64)));
  if(!managesHosts()){
   LinearLayout deviceHeader=new LinearLayout(this);deviceHeader.setOrientation(LinearLayout.VERTICAL);deviceHeader.setPadding(0,dp(12),0,dp(4));
   deviceHeading=new TextView(this);deviceHeading.setText(DeviceProfile.name(this));deviceHeading.setTextColor(0xffedf2ff);deviceHeading.setTextSize(21);deviceHeading.setTypeface(null,android.graphics.Typeface.BOLD);deviceHeading.setSingleLine(true);deviceHeading.setEllipsize(android.text.TextUtils.TruncateAt.END);deviceHeading.setContentDescription("Device name: "+DeviceProfile.name(this));deviceHeading.setOnClickListener(v->renameDevice());
   deviceHeader.addView(deviceHeading,new LinearLayout.LayoutParams(-1,-2));
   header.addView(deviceHeader,new LinearLayout.LayoutParams(0,-2,1));
  }
  machinesPill=button(managesHosts()?header:new LinearLayout(this),managesHosts()?"‹":"Manage",()->{if(managesHosts())finish();else showPairing(pairingScroll.getVisibility()!=View.VISIBLE);});machinesPill.setContentDescription(managesHosts()?"Back to space":"Manage paired machines");machinesPill.setBackground(card(0xd8101c34,0xff2b4373,21));machinesPill.setPadding(dp(managesHosts()?0:16),0,dp(managesHosts()?0:16),0);machinesPill.setTextSize(managesHosts()?18:14);LinearLayout.LayoutParams mlp=new LinearLayout.LayoutParams(managesHosts()?dp(44):-2,dp(42));mlp.rightMargin=dp(12);machinesPill.setLayoutParams(mlp);
  if(managesHosts()){ImageView logo=new ImageView(this);logo.setImageResource(R.drawable.ic_wispnet_logo);LinearLayout.LayoutParams ilp=new LinearLayout.LayoutParams(dp(34),dp(34));ilp.rightMargin=dp(12);header.addView(logo,ilp);TextView ht=new TextView(this);ht.setText("Wispnet · Hosts");ht.setTextColor(0xffedf2ff);ht.setTextSize(19);ht.setTypeface(null,android.graphics.Typeface.BOLD);header.addView(ht,new LinearLayout.LayoutParams(0,-2,1));}
  status=new TextView(this);status.setTextColor(0xffb8d4ff);status.setTextSize(14);status.setBackground(card(0xeb101d36,0xff2d4a7c,14));status.setPadding(dp(16),dp(12),dp(16),dp(12));FrameLayout.LayoutParams slp=new FrameLayout.LayoutParams(-1,-2,Gravity.BOTTOM);slp.setMargins(dp(20),0,dp(20),dp(20));status.setElevation(dp(12));status.setVisibility(View.GONE);screen.addView(status,slp);
  network=new TextView(this);identity=new TextView(this);identity.setTextColor(0xff8fa3c4);identity.setTextSize(12);identity.setTypeface(android.graphics.Typeface.MONOSPACE);identity.setBackground(card(0xff0d1526,0xff1e2d4a,14));identity.setPadding(dp(16),dp(12),dp(16),dp(12));identity.setTextIsSelectable(true);
  space=new FrameLayout(this);root.addView(space,new LinearLayout.LayoutParams(-1,0,1));
  servicesScroll=new ScrollView(this);list=new LinearLayout(this);list.setOrientation(LinearLayout.VERTICAL);list.setPadding(0,dp(12),0,dp(24));servicesScroll.addView(list);space.addView(servicesScroll,new FrameLayout.LayoutParams(-1,-1));servicesScroll.setVisibility(View.GONE);
  access=new SpaceToggle(this);access.setAllCaps(false);access.setTextColor(0xffe8edff);access.setTextSize(18);access.setOnClickListener(v->toggleSpace());space.addView(access);
  caption=new TextView(this);caption.setText("jump to wisp space");caption.setTextSize(21);caption.setLetterSpacing(0.05f);caption.setShadowLayer(dp(10),0,dp(2),0x883868c8);caption.setTextColor(0xffe8edff);caption.setGravity(Gravity.CENTER);FrameLayout.LayoutParams cap=new FrameLayout.LayoutParams(-1,dp(48),Gravity.CENTER);cap.topMargin=dp(82);space.addView(caption,cap);
  pairingScroll=new ScrollView(this);pairingScroll.setFillViewport(true);pairingScroll.setVerticalScrollBarEnabled(false);LinearLayout sheetWrap=new LinearLayout(this);sheetWrap.setOrientation(LinearLayout.VERTICAL);
  View scrim=new View(this);scrim.setMinimumHeight(dp(56));scrim.setOnClickListener(v->{if(managesHosts())finish();else showPairing(false);});sheetWrap.addView(scrim,new LinearLayout.LayoutParams(-1,0,1));
  pairingForm=new LinearLayout(this);pairingForm.setOrientation(LinearLayout.VERTICAL);pairingForm.setBackground(card(0xee0b1324,0xff294270,24));pairingForm.setPadding(dp(20),dp(12),dp(20),dp(24));LinearLayout.LayoutParams pflp=new LinearLayout.LayoutParams(-1,-2);pflp.bottomMargin=dp(12);sheetWrap.addView(pairingForm,pflp);pairingScroll.addView(sheetWrap);root.addView(pairingScroll,new LinearLayout.LayoutParams(-1,0,1));
  relayLabel=new TextView(this);relayLabel.setText("● Relay: connecting…");relayLabel.setTextColor(0xff94a6c4);relayLabel.setTextSize(12);relayLabel.setPadding(0,dp(6),0,dp(10));root.addView(relayLabel,new LinearLayout.LayoutParams(-1,-2));
  View handle=new View(this);handle.setBackground(card(0x668fa3c4,0x00000000,2));LinearLayout.LayoutParams hdlp=new LinearLayout.LayoutParams(dp(40),dp(4));hdlp.gravity=Gravity.CENTER_HORIZONTAL;hdlp.bottomMargin=dp(12);pairingForm.addView(handle,hdlp);
  LinearLayout sheetHeader=new LinearLayout(this);sheetHeader.setGravity(Gravity.CENTER_VERTICAL);LinearLayout.LayoutParams shlp=new LinearLayout.LayoutParams(-1,-2);shlp.bottomMargin=dp(12);pairingForm.addView(sheetHeader,shlp);
  sheetTitle=new TextView(this);sheetTitle.setText("Pair a machine");sheetTitle.setTextColor(0xffedf2ff);sheetTitle.setTextSize(20);sheetTitle.setTypeface(null,android.graphics.Typeface.BOLD);sheetHeader.addView(sheetTitle,new LinearLayout.LayoutParams(0,-2,1));
  Button closeSheet=button(sheetHeader,"✕",()->{if(managesHosts())finish();else showPairing(false);});closeSheet.setContentDescription("Close");closeSheet.setPadding(0,0,0,0);closeSheet.setTextSize(15);closeSheet.setBackground(card(0xff142038,0xff283b60,18));closeSheet.setLayoutParams(new LinearLayout.LayoutParams(dp(36),dp(36)));
  deviceName=button(pairingForm,"Device: "+DeviceProfile.name(this)+" · Rename",this::renameDevice);deviceName.setContentDescription("Rename device");
  machinesList=new LinearLayout(this);machinesList.setOrientation(LinearLayout.VERTICAL);pairingForm.addView(machinesList);
  addMachineBtn=button(pairingForm,"+ Pair another machine",()->{boolean open=addMachineBox.getVisibility()!=View.VISIBLE;addMachineBox.setVisibility(open?View.VISIBLE:View.GONE);addMachineBtn.setText(open?"Hide pairing options":"+ Pair another machine");});addMachineBtn.setVisibility(View.GONE);
  addMachineBox=new LinearLayout(this);addMachineBox.setOrientation(LinearLayout.VERTICAL);pairingForm.addView(addMachineBox);
  LinearLayout tabBar=new LinearLayout(this);tabBar.setBackground(card(0xff0d1526,0xff1e2d4a,14));tabBar.setPadding(dp(4),dp(4),dp(4),dp(4));LinearLayout.LayoutParams tblp=new LinearLayout.LayoutParams(-1,-2);tblp.bottomMargin=dp(14);addMachineBox.addView(tabBar,tblp);
  LinearLayout qrTab=new LinearLayout(this),codeTab=new LinearLayout(this);
  LinearLayout[] panels={qrTab,codeTab};String[] tabNames={"QR","Pairing Code"};Button[] tabs=new Button[2];
  java.util.function.IntConsumer selectTab=idx->{for(int i=0;i<2;i++){boolean sel=i==idx;tabs[i].setBackground(sel?card(0xff1f3666,0xff4b73c2,10): null);tabs[i].setTextColor(sel?0xffedf2ff:0xff8fa3c4);panels[i].setVisibility(sel?View.VISIBLE:View.GONE);}};
  for(int i=0;i<2;i++){final int idx=i;Button tb=new Button(this);tb.setText(tabNames[i]);tb.setAllCaps(false);tb.setTextSize(13);tb.setPadding(0,dp(10),0,dp(10));tb.setMinHeight(0);tb.setMinimumHeight(0);tb.setOnClickListener(v->selectTab.accept(idx));tabBar.addView(tb,new LinearLayout.LayoutParams(0,-2,1));tabs[i]=tb;panels[i].setOrientation(LinearLayout.VERTICAL);addMachineBox.addView(panels[i]);}
  Button scanBtn=button(qrTab,"Scan with camera",this::scanPairingQr);scanBtn.setTag("Scan pairing QR");
  Button photoBtn=button(qrTab,"Import from photos",()->{Intent pick=new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("image/*").addCategory(Intent.CATEGORY_OPENABLE);startActivityForResult(pick,3);});photoBtn.setTag("Import QR image");
  text(codeTab,"Paste a pairing code or import an invitation file.",14);invitation=field(codeTab,"Pairing code",InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_VARIATION_PASSWORD|InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
  button(codeTab,"Review pasted invitation",()->reviewInvitation(invitation.getText().toString()));
  button(codeTab,"Import invitation file",()->{Intent pick=new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("*/*").addCategory(Intent.CATEGORY_OPENABLE);startActivityForResult(pick,1);});
  selectTab.accept(0);
  reviewBox=new LinearLayout(this);reviewBox.setOrientation(LinearLayout.VERTICAL);reviewBox.setBackground(card(0xff101a30,0xff283b60,16));reviewBox.setPadding(dp(16),dp(8),dp(16),dp(12));LinearLayout.LayoutParams rblp=new LinearLayout.LayoutParams(-1,-2);rblp.topMargin=dp(6);rblp.bottomMargin=dp(10);reviewBox.setVisibility(View.GONE);addMachineBox.addView(reviewBox,rblp);
  text(reviewBox,"Verify target machine",15);label=field(reviewBox,"Machine name",InputType.TYPE_CLASS_TEXT);host=field(reviewBox,"Host ID",InputType.TYPE_CLASS_TEXT|InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
  hostVerified=new CheckBox(this);hostVerified.setTextColor(0xffc5d2e8);hostVerified.setPadding(dp(8),dp(10),dp(8),dp(10));hostVerified.setText("I compared this host ID with the target computer");LinearLayout.LayoutParams cvlp=new LinearLayout.LayoutParams(-1,-2);cvlp.bottomMargin=dp(8);reviewBox.addView(hostVerified,cvlp);
  host.addTextChangedListener(new android.text.TextWatcher(){public void beforeTextChanged(CharSequence s,int start,int count,int after){} public void onTextChanged(CharSequence s,int start,int before,int count){hostVerified.setChecked(false);if(s.length()>0){addMachineBox.setVisibility(View.VISIBLE);reviewBox.setVisibility(View.VISIBLE);}else reviewBox.setVisibility(View.GONE);} public void afterTextChanged(android.text.Editable s){}});
  button(reviewBox,"Request pairing",this::pair);
  LinearLayout.LayoutParams idlp=new LinearLayout.LayoutParams(-1,-2);idlp.topMargin=dp(8);addMachineBox.addView(identity,idlp);
  dockToggle(AccessService.enabled);showPairing(managesHosts());spaceShown=AccessService.enabled;servicesScroll.setVisibility(spaceShown?View.VISIBLE:View.GONE);
  if(checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)!=android.content.pm.PackageManager.PERMISSION_GRANTED)requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS},2);
  status.setText("");
  receiveInvitation(getIntent());
  work.execute(()->{try{Native.ensure(this);String peer=Native.command("identity").getString("peer");runOnUiThread(()->{identity.setText("Device ID\n"+peer);devicePeer=peer;updateDeviceHeading();if(status.getText().toString().startsWith("Preparing protected"))status.setText("Ready. Pair with each machine independently.");refresh();});}catch(Exception e){show("Cannot initialize protected identity: "+e.getMessage());}});
 }
 void updateDeviceHeading(){
  if(deviceHeading==null)return;
  String name=DeviceProfile.name(this);
  deviceHeading.setText(name+(devicePeer.isEmpty()?"":" · "+devicePeer.substring(0,8)));
  deviceHeading.setContentDescription("Device name: "+name+(devicePeer.isEmpty()?"":". Device ID: "+devicePeer));
 }
 int dp(int value){return Math.round(value*getResources().getDisplayMetrics().density);}
 android.graphics.drawable.GradientDrawable tint(int color,int radius){return card(color,0xff283b60,radius);}
 android.graphics.drawable.GradientDrawable card(int fill,int stroke,int radius){android.graphics.drawable.GradientDrawable d=new android.graphics.drawable.GradientDrawable();d.setColor(fill);d.setCornerRadius(dp(radius));d.setStroke(Math.max(1,dp(1)),stroke);return d;}
 void showPairing(boolean visible){
  if(closeServicePermissions!=null)closeServicePermissions.run();
  pairingScroll.setVisibility(visible?View.VISIBLE:View.GONE);space.setVisibility(visible?View.GONE:View.VISIBLE);
  if(access!=null&&access.getParent()==header)access.setVisibility(visible?View.GONE:View.VISIBLE);
  if(visible&&pairingForm!=null){pairingForm.setTranslationY(dp(36));pairingForm.setAlpha(0f);pairingForm.animate().translationY(0).alpha(1f).setDuration(200);}
 }
 void dockToggle(boolean docked){
  boolean paired=false;try{paired=store!=null&&store.all().length()>0;}catch(Exception ignored){}
  boolean first=!paired&&!managesHosts()&&!docked;
  ((SpaceToggle)access).plus=first;
  if(stars!=null)stars.hyperspace=docked;
  if(!managesHosts())machinesPill.setVisibility(paired?View.VISIBLE:View.GONE);
  ViewGroup parent=(ViewGroup)access.getParent();if(parent!=null)parent.removeView(access);
  if(docked&&!managesHosts())header.addView(access,new LinearLayout.LayoutParams(((SpaceToggle)access).dockedWidth(),dp(44)));
  else space.addView(access,new FrameLayout.LayoutParams(dp(108),dp(108),Gravity.CENTER));
  access.setTranslationY(docked&&!managesHosts()?dp(6):0);
  FrameLayout.LayoutParams cap=(FrameLayout.LayoutParams)caption.getLayoutParams();cap.topMargin=dp(82);caption.setLayoutParams(cap);
  caption.setText(first?"establish your wisp network":"jump to wisp space");
  access.setOnClickListener(v->{if(first)showPairing(true);else toggleSpace();});
  access.setVisibility((closeServicePermissions!=null||pairingScroll!=null&&pairingScroll.getVisibility()==View.VISIBLE)&&docked?View.GONE:View.VISIBLE);access.setText("");access.setContentDescription(first?"Establish your wisp network":docked?"Go offline":"Enter wisp space");access.setAlpha(1);access.setSelected(docked);access.setTag(first?"Establish your wisp network":docked?"Stop browser access":"Start browser access");caption.setVisibility(docked?View.GONE:View.VISIBLE);access.invalidate();
 }
 void holoReveal(){
  if(!android.animation.ValueAnimator.areAnimatorsEnabled())return;
  for(int i=0;i<list.getChildCount();i++){View v=list.getChildAt(i);v.setAlpha(0f);v.setTranslationY(dp(18));v.setScaleY(0.94f);v.animate().alpha(1f).translationY(0).scaleY(1f).setStartDelay(i*55L).setDuration(220);}
 }
 void toggleSpace(){
  if(starting||jumping||openingService)return;
  if(AccessService.enabled){stopService(new Intent(this,AccessService.class));lifecycle.postDelayed(this::syncSpace,40);return;}
  try{starting=true;enterRequested=true;startAt=SystemClock.elapsedRealtime();access.setEnabled(false);access.setText("…");startForegroundService(new Intent(this,AccessService.class));lifecycle.postDelayed(this::syncSpace,40);}catch(RuntimeException e){starting=false;enterRequested=false;access.setEnabled(true);show("Android could not start access. Try again.");}
 }
 void syncRelay(){
  if(relayLabel==null||!Native.ready||relayBusy||SystemClock.elapsedRealtime()-relayCheckedAt<2000)return;
  relayBusy=true;relayCheckedAt=SystemClock.elapsedRealtime();
  work.execute(()->{
   String state;try{state=Native.command("relay_status").getString("relay");}catch(Exception failure){state="unavailable";}
   final String observed=state;runOnUiThread(()->{relayBusy=false;relayLabel.setText("● Relay: "+observed);});
  });
 }
 void syncSpace(){
  syncRelay();
  if(access==null)return;boolean active=AccessService.enabled;
  if(!active&&openingService)resetServiceLaunch();
  if(active&&enterRequested&&!jumping){starting=false;enterRequested=false;jumping=true;hideNotice();caption.setVisibility(View.GONE);root.animate().alpha(0).setDuration(120);access.setEnabled(false);stars.jump(()->{jumping=false;root.animate().alpha(1).setDuration(200);if(AccessService.enabled){spaceShown=true;dockToggle(true);servicesScroll.setVisibility(View.VISIBLE);refresh();holoReveal();}access.setEnabled(true);});}
  else if(active&&!jumping&&!spaceShown){spaceShown=true;dockToggle(true);servicesScroll.setVisibility(View.VISIBLE);}
  else if(!active&&(spaceShown||jumping)){boolean wasShown=spaceShown;stars.cancel();if(wasShown)stars.dropOut();root.animate().cancel();root.setAlpha(1);jumping=false;spaceShown=false;servicesScroll.setVisibility(View.GONE);dockToggle(false);access.setEnabled(true);}
  if(starting&&SystemClock.elapsedRealtime()-startAt>15000){starting=false;enterRequested=false;dockToggle(false);access.setEnabled(true);stopService(new Intent(this,AccessService.class));show("Access could not start. Try again.");}
  updateNetworkFeedback(AccessService.networkStatus,active);
 }
 void updateNetworkFeedback(String current,boolean active){
  network.setText(current);
  if(active&&!current.equals(lastNetwork)){
   if(current.equals("Network available")){
    if(status.getText().toString().equals(lastNetwork)){status.setText("");hideNotice();}
   }else show(current);
  }
  lastNetwork=current;
 }
 void hideNotice(){
  noticeVersion++;noticeLeaving=false;lifecycle.removeCallbacks(dismissNotice);
  status.animate().cancel();status.animate().withEndAction(null);status.setVisibility(View.GONE);
 }
 void dismissNotice(){
  if(status.getVisibility()!=View.VISIBLE)return;
  int version=noticeVersion;noticeLeaving=true;
  status.animate().cancel();
  status.animate().translationY(status.getHeight()+((FrameLayout.LayoutParams)status.getLayoutParams()).bottomMargin).alpha(0)
   .setDuration(android.animation.ValueAnimator.areAnimatorsEnabled()?220:0)
   .setInterpolator(new android.view.animation.AccelerateInterpolator()).withEndAction(()->{if(version==noticeVersion)hideNotice();}).start();
 }
 void show(String value){runOnUiThread(()->{
  boolean entering=status.getVisibility()!=View.VISIBLE;
  int version=++noticeVersion;lifecycle.removeCallbacks(dismissNotice);
  status.animate().cancel();status.animate().withEndAction(null);
  boolean returning=noticeLeaving;noticeLeaving=false;
  status.setText(value);status.setVisibility(View.VISIBLE);status.announceForAccessibility(value);
  if(entering){status.setAlpha(0);status.setTranslationY(dp(64));}
  if(entering||returning)status.post(()->{
   if(version!=noticeVersion||status.getVisibility()!=View.VISIBLE)return;
   if(entering)status.setTranslationY(status.getHeight()+((FrameLayout.LayoutParams)status.getLayoutParams()).bottomMargin);
   status.animate().translationY(0).alpha(1).setDuration(android.animation.ValueAnimator.areAnimatorsEnabled()?220:0)
    .setInterpolator(new android.view.animation.DecelerateInterpolator()).start();
  });
  else{status.setAlpha(1);status.setTranslationY(0);}
  android.view.accessibility.AccessibilityManager accessibility=(android.view.accessibility.AccessibilityManager)getSystemService(ACCESSIBILITY_SERVICE);
  int timeout=accessibility.getRecommendedTimeoutMillis(4000,android.view.accessibility.AccessibilityManager.FLAG_CONTENT_TEXT);
  lifecycle.postDelayed(dismissNotice,timeout);
 });}
 void pair(){final String expected=host.getText().toString().trim(),document=invitation.getText().toString(),name=label.getText().toString().trim();
  if(!hostVerified.isChecked()){show("Compare the full host ID with the target computer, then confirm it before requesting pairing.");return;}
  if(expected.isEmpty()||document.isEmpty()||name.isEmpty()){show("Enter machine name, verified host ID and invitation");return;}
  requestPairing(expected,document,name);
 }
 void requestPairing(String expected,String document,String name){
  if(pairingBusy){show("A pairing request is already in progress. Please wait.");return;}
  pairingBusy=true;invitation.setText("");show("Requesting pairing from "+name+"… This may take up to a minute.");
  work.execute(()->{try{Native.ensure(this);Native.request(new JSONObject().put("op","pair").put("name",DeviceProfile.name(this)).put("host",expected).put("invitation",document));store.paired(expected,name);show("Pending owner approval · "+name);runOnUiThread(()->{if(managesHosts())finish();else{showPairing(false);render();}});}catch(Exception e){if(e instanceof Native.Failure&&"invitation_denied".equals(((Native.Failure)e).code))show("This host did not accept the invitation. Ask its owner for a new invitation.");else show("Pairing failed: "+e.getMessage()+". Scan a fresh invitation to try again.");}finally{runOnUiThread(()->pairingBusy=false);}});
 }
 void pairScannedQr(String document){
  if(pairingBusy){show("A pairing request is already in progress. Please wait.");return;}
  if(document.getBytes(java.nio.charset.StandardCharsets.UTF_8).length>8192){show("Invitation is too large. Ask for a new QR code.");return;}
  invitation.setText("");hostVerified.setChecked(false);show("Checking scanned invitation…");
  work.execute(()->{try{Native.ensure(this);JSONObject preview=Native.request(new JSONObject().put("op","preview_invitation").put("invitation",document));String target=preview.getString("host");runOnUiThread(()->{host.setText(target);String name=preview.optString("host_name","").trim();if(name.isEmpty())name="Host "+target.substring(0,8);label.setText(name);requestPairing(target,document,name);});}catch(Exception e){show("Cannot import invitation: "+e.getMessage());}});
 }

 void renameDevice(){
  EditText input=new EditText(this);input.setSingleLine(true);input.setText(DeviceProfile.name(this));input.setSelectAllOnFocus(true);input.setContentDescription("Device name");
  AlertDialog dialog=new AlertDialog.Builder(this).setTitle("Device name").setMessage("This name appears on your paired computers.").setView(input).setPositiveButton("Save",null).setNegativeButton("Cancel",null).create();
  dialog.setOnShowListener(ignored->dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(v->{String name=input.getText().toString().trim();
   if(!DeviceProfile.valid(name)){input.setError("Enter a name up to 128 UTF-8 bytes, without control characters");return;}
   try{DeviceProfile.save(this,name);deviceName.setText("Device: "+name+" · Rename");updateDeviceHeading();dialog.dismiss();show("Name saved. Updating paired computers…");work.execute(()->{boolean synced=DeviceProfile.syncAll(this);show(synced?"Device name updated":"Name saved. Paired computers will update when reachable; older hosts need an app update.");});}
   catch(Exception failure){input.setError(failure.getMessage());}
  }));dialog.show();
 }
 void refresh(){
  try{JSONObject hosts=store.all();for(java.util.Iterator<String> keys=hosts.keys();keys.hasNext();){String peer=keys.next();if(!refreshing.add(peer))continue;work.execute(()->{try{Native.ensure(this);JSONObject snapshot=Native.request(new JSONObject().put("op","catalog").put("host",peer));store.catalog(peer,snapshot);syncTelemetry();DeviceProfile.syncHost(this,peer);}catch(Exception e){try{store.failed(peer,e);}catch(Exception ignored){}}refreshing.remove(peer);runOnUiThread(this::render);});}render();}catch(Exception e){show("Cannot read saved hosts");}
 }
 void removeHost(String peer){
  work.execute(()->{try{store.remove(peer);boolean empty=store.all().length()==0;runOnUiThread(()->{if(empty&&AccessService.enabled)stopService(new Intent(this,AccessService.class));render();show("Machine removed; it will learn on its next contact");});}catch(Exception failure){show("Cannot complete removal: "+failure.getMessage());}});
 }
 void showHostDialog(String name,String peer,String statusText){
  String tel=peerTelemetry.get(peer);String info=tel!=null?statusText+" · "+tel:statusText;
  new AlertDialog.Builder(this).setTitle(name).setMessage("Host ID\n"+peer+"\n\n"+info).setPositiveButton("Refresh",(dialog,which)->refresh()).setNeutralButton("Remove",(dialog,which)->new AlertDialog.Builder(this).setTitle("Remove "+name+"?").setMessage("Remove this machine and close its access here? It will learn about removal when it next contacts this phone. You can pair again later.").setPositiveButton("Remove",(d2,w2)->removeHost(peer)).setNegativeButton("Cancel",null).show()).setNegativeButton("Done",null).show();
 }
 void render(){
  list.removeAllViews();ageUpdates.clear();machinesList.removeAllViews();
  LinearLayout heading=new LinearLayout(this);heading.setGravity(Gravity.CENTER_VERTICAL);LinearLayout.LayoutParams hlp=new LinearLayout.LayoutParams(-1,-2);hlp.bottomMargin=dp(12);list.addView(heading,hlp);
  TextView dirTitle=new TextView(this);dirTitle.setText("PAIRED MACHINES");dirTitle.setTextColor(0xff94a6c4);dirTitle.setTextSize(12);dirTitle.setLetterSpacing(0.12f);dirTitle.setTypeface(null,android.graphics.Typeface.BOLD);heading.addView(dirTitle,new LinearLayout.LayoutParams(0,-2,1));
  if(!managesHosts()){
   ViewGroup oldParent=(ViewGroup)machinesPill.getParent();if(oldParent!=null)oldParent.removeView(machinesPill);
   heading.addView(machinesPill);
  }
  Button reload=button(heading,"↻",this::refresh);reload.setContentDescription("Refresh services");reload.setTag("Refresh all hosts");reload.setPadding(0,0,0,0);reload.setTextSize(18);reload.setLayoutParams(new LinearLayout.LayoutParams(dp(44),dp(44)));
  try{JSONObject records=store.all();if(!managesHosts()&&!spaceShown&&!starting&&!jumping)dockToggle(AccessService.enabled);
   boolean hasRecords=records.length()>0;
   sheetTitle.setText(hasRecords?"Paired machines":"Pair a machine");
   addMachineBtn.setVisibility(hasRecords?View.VISIBLE:View.GONE);
   if(!hasRecords){((SpaceToggle)access).beacons=new int[0];access.invalidate();addMachineBox.setVisibility(View.VISIBLE);text(list,"Pair a machine to get started",16);return;}
   if(host.getText().length()==0){addMachineBox.setVisibility(View.GONE);addMachineBtn.setText("+ Pair another machine");}
   boolean anyAvailable=false,anyPending=false;int[] beacons=new int[records.length()];int bIdx=0;
   for(java.util.Iterator<String> keys=records.keys();keys.hasNext();){String peer=keys.next();JSONObject record=records.getJSONObject(peer);String name=record.getString("name"),state=record.optString("state"),statusText=record.optString("status");JSONObject catalog=record.optJSONObject("catalog");
    String tel=peerTelemetry.get(peer);
    String cardSub="available".equals(state)&&tel!=null?statusText+" · "+tel:statusText;
    int stateColor="available".equals(state)?0xff7be0b8:"approval_pending".equals(state)?0xfff0b85c:"host_denied".equals(state)?0xffff9aa8:0xff8fa3c4;
    beacons[bIdx++]=stateColor;
    if("available".equals(state))anyAvailable=true;else if("approval_pending".equals(state))anyPending=true;
    String hostTitle=name+" · "+peer.substring(0,8);
    Button machine=button(machinesList,hostTitle+"\n"+cardSub,()->showHostDialog(name,peer,statusText));
    machine.setText(styledCardText(hostTitle,cardSub,stateColor));machine.setGravity(Gravity.LEFT|Gravity.CENTER_VERTICAL);machine.setPadding(dp(18),dp(14),dp(18),dp(14));machine.setBackground(card(0xff101a30,"available".equals(state)?0xff2b4270:0xff223252,16));
    LinearLayout sector=new LinearLayout(this);sector.setOrientation(LinearLayout.VERTICAL);sector.setBackground(card(0xd80b1528,"available".equals(state)?0xff29497d:0xff1d2c4a,20));sector.setPadding(dp(16),dp(14),dp(16),dp(6));LinearLayout.LayoutParams secLp=new LinearLayout.LayoutParams(-1,-2);secLp.bottomMargin=dp(14);list.addView(sector,secLp);
    LinearLayout sectorHead=new LinearLayout(this);sectorHead.setGravity(Gravity.CENTER_VERTICAL);sectorHead.setPadding(dp(4),0,dp(4),dp(10));sectorHead.setOnClickListener(v->showHostDialog(name,peer,statusText));sector.addView(sectorHead,new LinearLayout.LayoutParams(-1,-2));
    TextView sName=new TextView(this);sName.setText("◈  "+name.toUpperCase()+" · "+peer.substring(0,8));sName.setTextColor(0xffedf4ff);sName.setTextSize(14);sName.setLetterSpacing(0.08f);sName.setTypeface(null,android.graphics.Typeface.BOLD);sectorHead.addView(sName,new LinearLayout.LayoutParams(0,-2,1));
    TextView sBadge=new TextView(this);sBadge.setText("available".equals(state)?("ONLINE"+(tel!=null?" · "+tel:"")):"approval_pending".equals(state)?"AWAITING APPROVAL":"host_denied".equals(state)?"ACCESS DENIED":"OFFLINE");sBadge.setTextColor("available".equals(state)?0xffedf4ff:stateColor);sBadge.setCompoundDrawablesWithIntrinsicBounds(new StatusDot(this,"available".equals(state)?0xff65ceff:"approval_pending".equals(state)?0xffe9b65c:"host_denied".equals(state)?0xffff9aa8:0xff7a8da8),null,null,null);sBadge.setCompoundDrawablePadding(dp(6));sBadge.setTextSize(11);sBadge.setLetterSpacing(0.06f);sBadge.setTypeface(null,android.graphics.Typeface.BOLD);sectorHead.addView(sBadge,new LinearLayout.LayoutParams(-2,-2));
    if(catalog==null){text(sector,name+" · "+("approval_pending".equals(state)?"Awaiting approval":"Unavailable"),14);continue;}
    JSONArray services=catalog.getJSONArray("services");if(services.length()==0)text(sector,name+" · No shared services",14);
    int serviceColumns=getResources().getConfiguration().screenWidthDp>=600?2:1;
    LinearLayout gridRow=null;
    for(int i=0;i<services.length();i++){String service=services.getJSONObject(i).getString("id");String incarnation=services.getJSONObject(i).optString("incarnation","");String display=services.getJSONObject(i).getString("display_name");String kind=services.getJSONObject(i).getString("kind");
     boolean allowed="http".equals(kind)&&incarnation.matches("[0-9a-f]{64}")&&!java.util.List.of("host_denied","approval_pending","state_changed","unverified").contains(state);
     String sub=!"available".equals(state)?name+" · "+("host_denied".equals(state)?"Denied":"approval_pending".equals(state)?"Pending":"unreachable".equals(state)?"Offline · cached":"Unavailable"):kind.toUpperCase();
     if(i%serviceColumns==0){gridRow=new LinearLayout(this);gridRow.setOrientation(LinearLayout.HORIZONTAL);sector.addView(gridRow,new LinearLayout.LayoutParams(-1,-2));}
     FrameLayout tile=new FrameLayout(this);tile.setBackground(card(0xff111e38,allowed?0xff32518a:0xff1d2842,16));
     LinearLayout labels=new LinearLayout(this);labels.setOrientation(LinearLayout.VERTICAL);labels.setGravity(Gravity.CENTER_VERTICAL);labels.setPadding(dp(12),0,dp(48),0);labels.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS);tile.addView(labels,new FrameLayout.LayoutParams(-1,-1));
     TextView title=new TextView(this);String titleText="✦  "+display+"   ↗";title.setText(styledCardText(titleText,sub,0xff94a6c4).subSequence(0,titleText.length()));title.setTextColor(0xffedf2ff);title.setTextSize(17);title.setSingleLine(true);title.setEllipsize(android.text.TextUtils.TruncateAt.END);labels.addView(title,new LinearLayout.LayoutParams(-1,-2));
     TextView subtitle=new TextView(this);subtitle.setText(sub);subtitle.setTextColor(allowed?0xff94a6c4:0xffff9aa8);subtitle.setTextSize(13);subtitle.setSingleLine(true);subtitle.setEllipsize(android.text.TextUtils.TruncateAt.END);labels.addView(subtitle,new LinearLayout.LayoutParams(-1,-2));labels.setAlpha(allowed?1:.45f);
     LinearLayout body=new LinearLayout(this);tile.addView(body,new FrameLayout.LayoutParams(-1,-1));
     Button row=button(body,"✦  "+display+"   ↗\n"+sub,()->open(peer,service,incarnation));row.setText("");row.setTag("Open "+name+" / "+service);row.setGravity(Gravity.LEFT|Gravity.CENTER_VERTICAL);row.setTextSize(17);row.setPadding(dp(12),dp(14),dp(48),dp(14));row.setBackgroundColor(android.graphics.Color.TRANSPARENT);row.setLayoutParams(new LinearLayout.LayoutParams(-1,-1));
     LinearLayout.LayoutParams params=new LinearLayout.LayoutParams(0,dp(88),1);params.bottomMargin=dp(10);if(i%serviceColumns<serviceColumns-1&&i+1<services.length())params.rightMargin=dp(10);gridRow.addView(tile,params);
     ImageButton gear=new ImageButton(this);gear.setImageResource(R.drawable.ic_service_settings);gear.setPadding(dp(12),dp(12),dp(12),dp(12));gear.setBackground(card(0x00111e38,0x00283b60,12));gear.setContentDescription("Permissions for "+display+" on "+name);gear.setTag("Configure "+name+" / "+service);
     gear.setOnClickListener(v->ServicePermissions.show(this,peer,service,display));tile.addView(gear,new FrameLayout.LayoutParams(dp(48),dp(48),Gravity.END|Gravity.CENTER_VERTICAL));
     row.setEnabled(allowed);row.setAlpha(allowed?1:.45f);row.setContentDescription(display+", "+name+", "+statusText);
     row.setOnLongClickListener(v->{new AlertDialog.Builder(this).setTitle(name).setMessage(cardSub+"\n\nHost ID\n"+peer+"\n\nDirectory age: "+Math.max(0,(System.currentTimeMillis()-record.optLong("fetched"))/1000)+" seconds").setPositiveButton("Done",null).show();return true;});
    }
   }
   ((SpaceToggle)access).beacons=beacons;access.invalidate();
  }catch(Exception e){show("Saved directory unavailable");}
 }
 public boolean dispatchTouchEvent(MotionEvent event){return openingService||super.dispatchTouchEvent(event);}
 void finishServiceLaunch(){
  if(!openingService||!serviceJumpDone||pendingServiceUrl==null)return;
  String url=pendingServiceUrl;resetServiceLaunch();
  if(!AccessService.enabled){show("Start browser access first");return;}
  hideNotice();
  Intent browser=new Intent(Intent.ACTION_VIEW,Uri.parse(url));browser.addCategory(Intent.CATEGORY_BROWSABLE);
  try{startActivity(browser);}catch(ActivityNotFoundException missing){show("Install a browser to open this service.");}
 }
 void resetServiceLaunch(){
  serviceLaunch++;openingService=false;serviceJumpDone=false;pendingServiceUrl=null;
  root.animate().cancel();root.setAlpha(1);stars.cancel();
 }
 void open(String peer,String service,String incarnation){
  if(openingService||jumping)return;
  if(!AccessService.enabled){show("Start browser access first");return;}
  openingService=true;serviceJumpDone=false;pendingServiceUrl=null;int launch=++serviceLaunch;
  show("Opening "+service+"…");
  stars.speedUp(()->{if(launch!=serviceLaunch)return;serviceJumpDone=true;finishServiceLaunch();});
  work.execute(()->{try{
   JSONObject response=Native.request(new JSONObject().put("op","open").put("host",peer).put("host_name",store.all().getJSONObject(peer).getString("name")).put("service",service).put("incarnation",incarnation));
   syncTelemetry();String url=response.optString("launch_url",response.getString("url"));
   runOnUiThread(()->{if(launch!=serviceLaunch)return;render();pendingServiceUrl=url;finishServiceLaunch();});
  }catch(Exception e){
   try{store.failed(peer,e);}catch(Exception ignored){}
   runOnUiThread(()->{if(launch!=serviceLaunch)return;resetServiceLaunch();show("Cannot open service: "+e.getMessage());render();});
  }});
 }
 void scanPairingQr(){
  if(!getPackageManager().hasSystemFeature(android.content.pm.PackageManager.FEATURE_CAMERA_ANY)){show("No camera available. Import a QR image or invitation file, or paste a pairing code.");return;}
  try{com.journeyapps.barcodescanner.ScanOptions options=new com.journeyapps.barcodescanner.ScanOptions().setCaptureActivity(PairingCaptureActivity.class).setDesiredBarcodeFormats(com.journeyapps.barcodescanner.ScanOptions.QR_CODE).setBeepEnabled(false).setBarcodeImageEnabled(false).setTimeout(60000).setPrompt("Scan your computer’s Wispnet QR to pair.");startActivityForResult(options.createScanIntent(this),2);}catch(RuntimeException unavailable){show("Camera unavailable. Import a QR image or invitation file, or paste a pairing code.");}
 }
 void readQrImage(Uri uri){
  invitation.setText("");hostVerified.setChecked(false);show("Reading QR image…");
  work.execute(()->{try{if(uri==null||!"content".equals(uri.getScheme()))throw new IOException();try(InputStream input=getContentResolver().openInputStream(uri)){String document=PairingQr.decode(input);runOnUiThread(()->reviewInvitation(document));}}catch(Exception invalid){show("Cannot read pairing QR image. Import an invitation file or paste its pairing code instead.");}});
 }
 void reviewInvitation(String document){
  if(document.getBytes(java.nio.charset.StandardCharsets.UTF_8).length>8192){invitation.setText("");show("Invitation is too large. Ask for a new invitation file.");return;}
  hostVerified.setChecked(false);invitation.setText("");show("Reviewing invitation…");
  work.execute(()->{try{Native.ensure(this);JSONObject preview=Native.request(new JSONObject().put("op","preview_invitation").put("invitation",document));runOnUiThread(()->{host.setText(preview.optString("host"));String machine=preview.optString("host_name","").trim();if(!machine.isEmpty())label.setText(machine);invitation.setText(document);show("Invitation ready for review. Compare the full host ID on the target computer. Importing does not request pairing or grant access.");});}catch(Exception e){show("Cannot import invitation: "+e.getMessage());}});
 }
 void importInvitation(InputStream in)throws IOException{byte[] bytes=in.readNBytes(8193);if(bytes.length>8192)throw new IOException();String document=new String(bytes,java.nio.charset.StandardCharsets.UTF_8);runOnUiThread(()->reviewInvitation(document));}
 void readInvitation(Uri uri){
  invitation.setText("");hostVerified.setChecked(false);show("Reading invitation file…");
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
 void configureSystemBars(){getWindow().setStatusBarColor(android.graphics.Color.TRANSPARENT);getWindow().setStatusBarContrastEnforced(false);WindowInsetsController controller=getWindow().getInsetsController();if(controller!=null){controller.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);controller.setSystemBarsAppearance(0,WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS);controller.show(WindowInsets.Type.statusBars());controller.hide(WindowInsets.Type.navigationBars());}}
 public void onWindowFocusChanged(boolean focused){super.onWindowFocusChanged(focused);if(focused)configureSystemBars();}
 protected void onResume(){super.onResume();configureSystemBars();if(store!=null&&!managesHosts()&&!jumping&&!starting)dockToggle(AccessService.enabled);lifecycle.post(updateAccess);syncSpace();}
 protected void onPause(){if(openingService)resetServiceLaunch();lifecycle.removeCallbacks(updateAccess);super.onPause();}
 protected void onDestroy(){hideNotice();root.animate().cancel();stars.cancel();access.animate().cancel();work.shutdown();super.onDestroy();}
}
