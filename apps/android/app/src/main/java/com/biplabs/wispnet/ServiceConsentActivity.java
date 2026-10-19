package com.biplabs.wispnet;

import android.app.*;
import android.content.*;
import android.os.*;
import android.view.*;
import org.json.*;

/** Non-exported; only Wispnet-created immutable PendingIntents can open consent. */
public final class ServiceConsentActivity extends Activity {
 protected void onCreate(Bundle state){
  super.onCreate(state);getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);
  try{
   Intent input=getIntent();int uid=input.getIntExtra("uid",-1);String pkg=input.getStringExtra("package_name"),identity=input.getStringExtra("identity");
   if(!AppGrants.identity(this,uid,pkg).equals(identity))throw new SecurityException();
   String host=input.getStringExtra("host_id"),service=input.getStringExtra("service_id"),incarnation=input.getStringExtra("incarnation");
   String id=AppGrants.key(identity,host,service,incarnation);
   String label=getPackageManager().getApplicationLabel(getPackageManager().getApplicationInfo(pkg,0)).toString();
   AlertDialog dialog=new AlertDialog.Builder(this).setTitle("Allow service access?")
    .setMessage(label+" ("+pkg+") wants to use "+service+" on "+input.getStringExtra("host_name")+".\n\nHost ID\n"+host+"\n\nThis permits HTTP requests to this service. Wispnet will go online. You can revoke access using the gear beside this service.")
    .setPositiveButton("Allow",(d,w)->{
     try{
      if(!AppGrants.identity(this,uid,pkg).equals(identity))throw new SecurityException();
      new AppGrants(this).approve(id,new JSONObject().put("package_name",pkg).put("uid",uid).put("identity",identity)
       .put("host_id",host).put("service_id",service).put("incarnation",incarnation).put("label",label));
      if(!AccessService.enabled)startForegroundService(new Intent(this,AccessService.class));
      setResult(RESULT_OK);
     }catch(Exception failure){setResult(RESULT_CANCELED);}finish();
    }).setNegativeButton("Deny",(d,w)->{setResult(RESULT_CANCELED);finish();}).setOnCancelListener(d->finish()).create();
   dialog.setOnShowListener(d->{dialog.getButton(AlertDialog.BUTTON_POSITIVE).setFilterTouchesWhenObscured(true);dialog.getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);});dialog.show();
  }catch(Exception invalid){setResult(RESULT_CANCELED);finish();}
 }
}
