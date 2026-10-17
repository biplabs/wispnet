package com.biplabs.wispnet;
import android.content.Context;
import android.graphics.*;
import android.widget.Button;
import android.view.accessibility.AccessibilityNodeInfo;
/** A real accessible switch with a compact custom dark appearance. */
final class SpaceToggle extends Button {
 final Paint paint=new Paint(Paint.ANTI_ALIAS_FLAG);
 SpaceToggle(Context context){super(context);setBackgroundColor(Color.TRANSPARENT);setMinWidth(0);setMinHeight(0);}
 protected void onDraw(Canvas c){float d=getResources().getDisplayMetrics().density,w=64*d,h=36*d,x=(getWidth()-w)/2,y=(getHeight()-h)/2;
  paint.setColor(isSelected()?0xff5679ef:0xff253149);c.drawRoundRect(x,y,x+w,y+h,h/2,h/2,paint);
  paint.setColor(isEnabled()?0xfff2f6ff:0xff929eb4);float r=13*d;c.drawCircle(x+(isSelected()?w-18*d:18*d),y+h/2,r,paint);
 }
 public void setSelected(boolean value){super.setSelected(value);invalidate();}
 public void onInitializeAccessibilityNodeInfo(AccessibilityNodeInfo info){super.onInitializeAccessibilityNodeInfo(info);info.setClassName("android.widget.Switch");info.setCheckable(true);info.setChecked(isSelected());}
}
