package com.biplabs.wispnet;
import android.content.Context;
import android.graphics.*;
import android.widget.Button;
import android.view.accessibility.AccessibilityNodeInfo;
/** Hyperdrive Navicomputer Core in standby, active Go offline control when docked, or first-run plus core. */
final class SpaceToggle extends Button {
 final Paint paint=new Paint(Paint.ANTI_ALIAS_FLAG);
 final RectF arc=new RectF();
 boolean plus=false;
 int[] beacons=new int[0];
 SpaceToggle(Context context){super(context);setBackgroundColor(Color.TRANSPARENT);setMinWidth(0);setMinHeight(0);}
 void offlineText(float density){
  paint.setTextSize(11.5f*density);paint.setTypeface(Typeface.create(Typeface.DEFAULT,Typeface.BOLD));paint.setLetterSpacing(0.08f);
 }
 int dockedWidth(){
  float d=getResources().getDisplayMetrics().density;offlineText(d);
  return (int)Math.ceil(paint.measureText("GO OFFLINE")+48*d);
 }
 protected void onDraw(Canvas c){
  float d=getResources().getDisplayMetrics().density,cx=getWidth()/2f,cy=getHeight()/2f;
  boolean on=isSelected();
  if(plus||!on){
   float r=38*d,ring=r+8*d;
   paint.setStyle(Paint.Style.FILL);paint.setColor(0x284582ff);c.drawCircle(cx,cy,r+5*d,paint);
   paint.setColor(0xff0f1c36);c.drawCircle(cx,cy,r,paint);
   paint.setStyle(Paint.Style.STROKE);paint.setStrokeWidth(1.6f*d);paint.setColor(0xff528ef2);c.drawCircle(cx,cy,r,paint);
   arc.set(cx-ring,cy-ring,cx+ring,cy+ring);paint.setStrokeWidth(1.3f*d);paint.setColor(0x8865ceff);
   for(int a=0;a<360;a+=90)c.drawArc(arc,a+18,54,false,paint);
   if(plus){
    float arm=13*d;paint.setStrokeCap(Paint.Cap.ROUND);paint.setStrokeWidth(3.2f*d);paint.setColor(0xffedf4ff);
    c.drawLine(cx-arm,cy,cx+arm,cy,paint);c.drawLine(cx,cy-arm,cx,cy+arm,paint);return;
   }
   paint.setStyle(Paint.Style.STROKE);paint.setStrokeWidth(1.1f*d);paint.setColor(0x4465ceff);c.drawCircle(cx,cy,r*.62f,paint);
   paint.setStyle(Paint.Style.FILL);paint.setColor(0x3365ceff);c.drawCircle(cx,cy,11*d,paint);
   paint.setColor(isEnabled()?0xff65ceff:0xff7a8da8);c.drawCircle(cx,cy,4.5f*d,paint);
   paint.setColor(0xfff2fbff);c.drawCircle(cx,cy,2f*d,paint);
   paint.setStyle(Paint.Style.STROKE);paint.setStrokeCap(Paint.Cap.ROUND);paint.setStrokeWidth(2.2f*d);paint.setColor(0xffdff4ff);
   c.drawLine(cx-7*d,cy-10*d,cx,cy-16*d,paint);c.drawLine(cx+7*d,cy-10*d,cx,cy-16*d,paint);
   for(int i=0;i<beacons.length;i++){
    double ang=-Math.PI/2+i*(2*Math.PI/Math.max(1,beacons.length));
    float bx=cx+(float)Math.cos(ang)*ring,by=cy+(float)Math.sin(ang)*ring;
    paint.setStyle(Paint.Style.FILL);paint.setColor(0xff070b14);c.drawCircle(bx,by,5.2f*d,paint);
    paint.setColor(beacons[i]);c.drawCircle(bx,by,3.6f*d,paint);
   }
   return;
  }
  float w=getWidth()-4*d,h=38*d,x=2*d,y=(getHeight()-h)/2f;
  paint.setStyle(Paint.Style.FILL);paint.setColor(0x334582ff);c.drawRoundRect(x-2*d,y-2*d,x+w+2*d,y+h+2*d,(h+4*d)/2,(h+4*d)/2,paint);
  paint.setColor(0xd812254a);c.drawRoundRect(x,y,x+w,y+h,h/2,h/2,paint);
  paint.setStyle(Paint.Style.STROKE);paint.setStrokeWidth(1.4f*d);paint.setColor(0xff5b9cf7);c.drawRoundRect(x,y,x+w,y+h,h/2,h/2,paint);
  offlineText(d);
  float textWidth=paint.measureText("GO OFFLINE");
  float contentWidth=8.4f*d+8*d+textWidth;
  float bx=(getWidth()-contentWidth)/2f+4.2f*d,by=y+h/2f;
  paint.setStyle(Paint.Style.FILL);paint.setColor(0x4465ceff);c.drawCircle(bx,by,7.5f*d,paint);
  paint.setColor(0xff65ceff);c.drawCircle(bx,by,4.2f*d,paint);
  paint.setColor(0xfff4fcff);c.drawCircle(bx,by,1.8f*d,paint);
  paint.setColor(0xffedf4ff);offlineText(d);
  Paint.FontMetrics fm=paint.getFontMetrics();c.drawText("GO OFFLINE",bx+12.2f*d,by-(fm.ascent+fm.descent)/2f,paint);
 }
 public void setSelected(boolean value){super.setSelected(value);invalidate();}
 public void onInitializeAccessibilityNodeInfo(AccessibilityNodeInfo info){super.onInitializeAccessibilityNodeInfo(info);info.setClassName(plus?"android.widget.Button":"android.widget.Switch");info.setCheckable(!plus);info.setChecked(!plus&&isSelected());}
}
