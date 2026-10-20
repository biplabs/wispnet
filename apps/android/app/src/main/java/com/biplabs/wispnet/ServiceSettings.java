package com.biplabs.wispnet;

import android.view.*;
import android.widget.*;
import org.json.*;

/** Service settings use the same in-screen sheet as machine pairing. */
final class ServiceSettings {
 static void show(MainActivity activity,String host,String service,String display){
  activity.showPairing(false);
  ScrollView scroll=new ScrollView(activity);scroll.setFillViewport(true);scroll.setVerticalScrollBarEnabled(false);scroll.setTag("Service settings sheet");
  LinearLayout wrap=new LinearLayout(activity);wrap.setOrientation(LinearLayout.VERTICAL);scroll.addView(wrap);
  View scrim=new View(activity);scrim.setMinimumHeight(activity.dp(56));wrap.addView(scrim,new LinearLayout.LayoutParams(-1,0,1));
  LinearLayout sheet=new LinearLayout(activity);sheet.setOrientation(LinearLayout.VERTICAL);sheet.setBackground(activity.card(0xee0b1324,0xff294270,24));sheet.setPadding(activity.dp(20),activity.dp(12),activity.dp(20),activity.dp(24));
  LinearLayout.LayoutParams sheetParams=new LinearLayout.LayoutParams(-1,-2);sheetParams.bottomMargin=activity.dp(12);wrap.addView(sheet,sheetParams);
  View handle=new View(activity);handle.setBackground(activity.card(0x668fa3c4,0x00000000,2));LinearLayout.LayoutParams handleParams=new LinearLayout.LayoutParams(activity.dp(40),activity.dp(4));handleParams.gravity=Gravity.CENTER_HORIZONTAL;handleParams.bottomMargin=activity.dp(12);sheet.addView(handle,handleParams);
  android.window.OnBackInvokedCallback[] back={null};
  Runnable close=()->{
   activity.closeServiceSettings=null;activity.root.removeView(scroll);
   if(back[0]!=null)activity.getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(back[0]);
   activity.showPairing(false);
  };
  activity.closeServiceSettings=close;back[0]=close::run;
  activity.getOnBackInvokedDispatcher().registerOnBackInvokedCallback(android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT,back[0]);scrim.setOnClickListener(v->close.run());
  LinearLayout header=new LinearLayout(activity);header.setGravity(Gravity.CENTER_VERTICAL);LinearLayout.LayoutParams headerParams=new LinearLayout.LayoutParams(-1,-2);headerParams.bottomMargin=activity.dp(12);sheet.addView(header,headerParams);
  TextView title=new TextView(activity);title.setText(display);title.setTextColor(0xffedf2ff);title.setTextSize(20);title.setTypeface(null,android.graphics.Typeface.BOLD);title.setSingleLine(true);title.setEllipsize(android.text.TextUtils.TruncateAt.END);header.addView(title,new LinearLayout.LayoutParams(0,-2,1));
  Button dismiss=activity.button(header,"✕",close);dismiss.setContentDescription("Close service settings");dismiss.setPadding(0,0,0,0);dismiss.setTextSize(15);dismiss.setBackground(activity.card(0xff142038,0xff283b60,18));dismiss.setLayoutParams(new LinearLayout.LayoutParams(activity.dp(36),activity.dp(36)));
  TextView detail=new TextView(activity);detail.setText(host==null?"Registry port and websites allowed to use your services. Changes reconnect active sessions.":"Gateway port on this device. Leave blank to keep an automatically assigned port. Changes reconnect active sessions.");detail.setTextColor(0xff94a6c4);detail.setPadding(0,activity.dp(8),0,activity.dp(16));sheet.addView(detail);
  EditText port=new EditText(activity);port.setTextColor(0xffedf2ff);port.setHintTextColor(0xff94a6c4);port.setInputType(android.text.InputType.TYPE_CLASS_NUMBER);port.setHint(host==null?"Registry port":"Automatic");port.setContentDescription(host==null?"Registry port":"Service gateway port");sheet.addView(port);
  EditText origins=new EditText(activity);origins.setTextColor(0xffedf2ff);origins.setHintTextColor(0xff94a6c4);origins.setHint("Allowed website origins, one per line");origins.setContentDescription("Allowed website origins");if(host==null)sheet.addView(origins);
  TextView feedback=new TextView(activity);feedback.setTextIsSelectable(true);feedback.setTextColor(0xffb8d4ff);sheet.addView(feedback);
  Button save=activity.button(sheet,"Save",()->{});save.setEnabled(false);
  final JSONObject[] current={null};
  activity.work.execute(()->{try{Native.ensure(activity);JSONObject data=Native.command("gateway_settings");JSONObject settings=data.getJSONObject("settings");current[0]=settings;String selected="";
   if(host==null)selected=String.valueOf(settings.getInt("registry_port"));else{JSONArray ports=settings.getJSONArray("service_ports");for(int i=0;i<ports.length();i++){JSONObject r=ports.getJSONObject(i);if(host.equals(r.getString("host"))&&service.equals(r.getString("service")))selected=String.valueOf(r.getInt("port"));}}
   String address=host==null?"http://registry.wispnet.localhost:"+settings.getInt("registry_port"):"";if(host!=null){JSONArray routes=data.getJSONArray("routes");for(int i=0;i<routes.length();i++){JSONObject r=routes.getJSONObject(i);if(host.equals(r.getString("host"))&&service.equals(r.getString("service")))address=r.getString("url");}}final String currentUrl=address;
   final String value=selected;StringBuilder allowed=new StringBuilder();JSONArray items=settings.getJSONArray("allowed_origins");for(int i=0;i<items.length();i++){if(i>0)allowed.append("\n");allowed.append(items.getString(i));}
   activity.runOnUiThread(()->{port.setText(value);origins.setText(allowed.toString());feedback.setText(currentUrl);save.setEnabled(true);});
  }catch(Exception failure){activity.runOnUiThread(()->feedback.setText("Could not load access settings."));}});
  save.setOnClickListener(v->{try{
   String raw=port.getText().toString().trim();int number=raw.isEmpty()?0:Integer.parseInt(raw);if((host==null||number!=0)&&(number<1024||number>65535))throw new Exception("Port must be between 1024 and 65535.");
   JSONObject updated=new JSONObject(current[0].toString());
   if(host==null){updated.put("registry_port",number);JSONArray allowed=new JSONArray();for(String line:origins.getText().toString().split("\n")){if(!line.trim().isEmpty())allowed.put(line.trim());}updated.put("allowed_origins",allowed);}
   else{JSONArray ports=new JSONArray(),old=updated.getJSONArray("service_ports");for(int i=0;i<old.length();i++){JSONObject r=old.getJSONObject(i);if(!host.equals(r.getString("host"))||!service.equals(r.getString("service")))ports.put(r);}if(number!=0)ports.put(new JSONObject().put("host",host).put("service",service).put("port",number));updated.put("service_ports",ports);}
   save.setEnabled(false);activity.work.execute(()->{try{Native.request(new JSONObject().put("op","configure_access").put("settings",updated));activity.runOnUiThread(close);}catch(Exception failure){activity.runOnUiThread(()->{feedback.setText(failure.getMessage());save.setEnabled(true);});}});
  }catch(Exception failure){feedback.setText(failure.getMessage());}});
  activity.space.setVisibility(View.GONE);if(activity.access.getParent()==activity.header)activity.access.setVisibility(View.GONE);
  activity.root.addView(scroll,activity.root.indexOfChild(activity.relayLabel),new LinearLayout.LayoutParams(-1,0,1));
  sheet.setTranslationY(activity.dp(36));sheet.setAlpha(0f);sheet.animate().translationY(0).alpha(1f).setDuration(200);
 }
}
