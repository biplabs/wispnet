package com.biplabs.wispnet;

import android.view.*;
import android.widget.*;
import org.json.*;

/** Service settings use the same in-screen sheet as machine pairing. */
final class ServicePermissions {
 static void show(MainActivity activity,String host,String service,String display){
  activity.showPairing(false);
  ScrollView scroll=new ScrollView(activity);scroll.setFillViewport(true);scroll.setVerticalScrollBarEnabled(false);scroll.setTag("Service permissions sheet");
  LinearLayout wrap=new LinearLayout(activity);wrap.setOrientation(LinearLayout.VERTICAL);scroll.addView(wrap);
  View scrim=new View(activity);scrim.setMinimumHeight(activity.dp(56));wrap.addView(scrim,new LinearLayout.LayoutParams(-1,0,1));
  LinearLayout sheet=new LinearLayout(activity);sheet.setOrientation(LinearLayout.VERTICAL);sheet.setBackground(activity.card(0xee0b1324,0xff294270,24));sheet.setPadding(activity.dp(20),activity.dp(12),activity.dp(20),activity.dp(24));
  LinearLayout.LayoutParams sheetParams=new LinearLayout.LayoutParams(-1,-2);sheetParams.bottomMargin=activity.dp(12);wrap.addView(sheet,sheetParams);
  View handle=new View(activity);handle.setBackground(activity.card(0x668fa3c4,0x00000000,2));LinearLayout.LayoutParams handleParams=new LinearLayout.LayoutParams(activity.dp(40),activity.dp(4));handleParams.gravity=Gravity.CENTER_HORIZONTAL;handleParams.bottomMargin=activity.dp(12);sheet.addView(handle,handleParams);
  android.window.OnBackInvokedCallback[] back={null};
  Runnable close=()->{
   activity.closeServicePermissions=null;activity.root.removeView(scroll);
   if(back[0]!=null)activity.getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(back[0]);
   activity.showPairing(false);
  };
  activity.closeServicePermissions=close;back[0]=close::run;
  activity.getOnBackInvokedDispatcher().registerOnBackInvokedCallback(android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT,back[0]);scrim.setOnClickListener(v->close.run());
  LinearLayout header=new LinearLayout(activity);header.setGravity(Gravity.CENTER_VERTICAL);LinearLayout.LayoutParams headerParams=new LinearLayout.LayoutParams(-1,-2);headerParams.bottomMargin=activity.dp(12);sheet.addView(header,headerParams);
  TextView title=new TextView(activity);title.setText(display);title.setTextColor(0xffedf2ff);title.setTextSize(20);title.setTypeface(null,android.graphics.Typeface.BOLD);title.setSingleLine(true);title.setEllipsize(android.text.TextUtils.TruncateAt.END);header.addView(title,new LinearLayout.LayoutParams(0,-2,1));
  Button dismiss=activity.button(header,"✕",close);dismiss.setContentDescription("Close service permissions");dismiss.setPadding(0,0,0,0);dismiss.setTextSize(15);dismiss.setBackground(activity.card(0xff142038,0xff283b60,18));dismiss.setLayoutParams(new LinearLayout.LayoutParams(activity.dp(36),activity.dp(36)));
  TextView heading=new TextView(activity);heading.setText("App permissions");heading.setTextColor(0xffedf2ff);heading.setTextSize(16);sheet.addView(heading);
  TextView detail=new TextView(activity);detail.setText("Apps allowed to use this service on this phone.");detail.setTextColor(0xff94a6c4);detail.setTextSize(14);detail.setPadding(0,activity.dp(8),0,activity.dp(16));sheet.addView(detail);
  LinearLayout grants=new LinearLayout(activity);grants.setOrientation(LinearLayout.VERTICAL);sheet.addView(grants);
  Runnable[] render={null};render[0]=()->{
   if(activity.isFinishing()||activity.isDestroyed()||scroll.getParent()==null)return;
   grants.removeAllViews();int count=0;
   try{JSONObject all=new AppGrants(activity).all();java.util.Iterator<String> keys=all.keys();
    while(keys.hasNext()){
     String id=keys.next();JSONObject grant=all.getJSONObject(id);
     if(!host.equals(grant.getString("host_id"))||!service.equals(grant.getString("service_id")))continue;
     count++;
     LinearLayout card=new LinearLayout(activity);card.setOrientation(LinearLayout.VERTICAL);card.setBackground(activity.card(0xff101a30,0xff283b60,14));card.setPadding(activity.dp(16),activity.dp(14),activity.dp(16),activity.dp(4));LinearLayout.LayoutParams cardParams=new LinearLayout.LayoutParams(-1,-2);cardParams.bottomMargin=activity.dp(10);grants.addView(card,cardParams);
     TextView label=new TextView(activity);label.setText(grant.optString("label"));label.setTextColor(0xffedf2ff);label.setTextSize(16);label.setTypeface(null,android.graphics.Typeface.BOLD);card.addView(label);
     TextView pkg=new TextView(activity);pkg.setText(grant.getString("package_name"));pkg.setTextColor(0xff94a6c4);pkg.setTextSize(12);pkg.setPadding(0,activity.dp(4),0,activity.dp(12));card.addView(pkg);
     LinearLayout confirmation=new LinearLayout(activity);confirmation.setOrientation(LinearLayout.VERTICAL);confirmation.setVisibility(View.GONE);
     Button revoke=activity.button(card,"Revoke",()->{});revoke.setTextColor(0xffff9aa8);revoke.setContentDescription("Revoke "+grant.getString("package_name"));card.addView(confirmation);
     TextView warning=new TextView(activity);warning.setText("Disconnect this app? Active connections will close. Other apps using this service may need to reconnect.");warning.setTextColor(0xffb8d4ff);warning.setTextSize(14);warning.setPadding(0,0,0,activity.dp(12));confirmation.addView(warning);
     Button confirm=activity.button(confirmation,"Revoke",()->{});confirm.setTextColor(0xffff9aa8);
     Button cancel=activity.button(confirmation,"Cancel",()->{confirmation.setVisibility(View.GONE);revoke.setVisibility(View.VISIBLE);});
     revoke.setOnClickListener(v->{revoke.setVisibility(View.GONE);confirmation.setVisibility(View.VISIBLE);});
     confirm.setOnClickListener(v->{confirm.setEnabled(false);cancel.setEnabled(false);activity.work.execute(()->{
      try{new AppGrants(activity).revoke(id);}catch(Exception failure){activity.runOnUiThread(()->Toast.makeText(activity,"Could not complete revocation; access was stopped.",Toast.LENGTH_LONG).show());}
      activity.runOnUiThread(render[0]);
     });});
    }
    if(count==0){TextView empty=new TextView(activity);empty.setText("No apps have access to this service.");empty.setTextColor(0xff94a6c4);empty.setTextSize(14);empty.setBackground(activity.card(0xff101a30,0xff283b60,14));empty.setPadding(activity.dp(16),activity.dp(18),activity.dp(16),activity.dp(18));grants.addView(empty);}
   }catch(Exception invalid){TextView error=new TextView(activity);error.setText("Cannot read app permissions.");error.setTextColor(0xffff9aa8);grants.addView(error);}
  };
  activity.space.setVisibility(View.GONE);if(activity.access.getParent()==activity.header)activity.access.setVisibility(View.GONE);
  activity.root.addView(scroll,activity.root.indexOfChild(activity.relayLabel),new LinearLayout.LayoutParams(-1,0,1));
  render[0].run();sheet.setTranslationY(activity.dp(36));sheet.setAlpha(0f);sheet.animate().translationY(0).alpha(1f).setDuration(200);
 }
}
