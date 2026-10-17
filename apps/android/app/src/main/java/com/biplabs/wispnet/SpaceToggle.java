package com.biplabs.wispnet;
import android.content.Context;
import android.graphics.*;
import android.widget.Button;
import android.view.accessibility.AccessibilityNodeInfo;
/** A real accessible switch with a compact custom dark appearance. */
final class SpaceToggle extends Button {
 final Paint paint=new Paint(Paint.ANTI_ALIAS_FLAG);
 SpaceToggle(Context context){super(context);setBackgroundColor(Color.TRANSPARENT);setMinWidth(0);setMinHeight(0);}
 protected void onDraw(Canvas c){float d=getResources().getDisplayMetrics().density,w=66*d,h=36*d,x=(getWidth()-w)/2,y=(getHeight()-h)/2;
  boolean on=isSelected();
  if(on){paint.setStyle(Paint.Style.FILL);paint.setColor(0x334f83ff);c.drawRoundRect(x-3*d,y-3*d,x+w+3*d,y+h+3*d,(h+6*d)/2,(h+6*d)/2,paint);}
  paint.setStyle(Paint.Style.FILL);paint.setColor(on?0xff3b66e6:0xff141f36);c.drawRoundRect(x,y,x+w,y+h,h/2,h/2,paint);
  paint.setStyle(Paint.Style.STROKE);paint.setStrokeWidth(1.4f*d);paint.setColor(on?0xff79b4ff:0xff30456e);c.drawRoundRect(x,y,x+w,y+h,h/2,h/2,paint);
  float tx=x+(on?w-18*d:18*d),ty=y+h/2,r=13*d;
  paint.setStyle(Paint.Style.FILL);paint.setColor(0x38000000);c.drawCircle(tx,ty+1.2f*d,r,paint);
  paint.setColor(isEnabled()?0xfff4f8ff:0xff929eb4);c.drawCircle(tx,ty,r,paint);
  if(on&&isEnabled()){paint.setColor(0xff65ceff);c.drawCircle(tx,ty,3.5f*d,paint);}
 }
 public void setSelected(boolean value){super.setSelected(value);invalidate();}
 public void onInitializeAccessibilityNodeInfo(AccessibilityNodeInfo info){super.onInitializeAccessibilityNodeInfo(info);info.setClassName("android.widget.Switch");info.setCheckable(true);info.setChecked(isSelected());}
}
