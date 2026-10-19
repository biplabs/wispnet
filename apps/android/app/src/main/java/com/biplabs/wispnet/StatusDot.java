package com.biplabs.wispnet;

import android.content.Context;
import android.graphics.*;
import android.graphics.drawable.Drawable;

/** Shared status-dot shape matching the desktop glow, core and highlight. */
final class StatusDot extends Drawable {
 private final Paint paint=new Paint(Paint.ANTI_ALIAS_FLAG);
 private final int size,color;
 private int alpha=255;
 StatusDot(Context context,int color){this.size=Math.round(16*context.getResources().getDisplayMetrics().density);this.color=color;}
 public int getIntrinsicWidth(){return size;}
 public int getIntrinsicHeight(){return size;}
 public void draw(Canvas canvas){
  Rect bounds=getBounds();float unit=Math.min(bounds.width(),bounds.height())/16f,cx=bounds.exactCenterX(),cy=bounds.exactCenterY();
  paint.setColor(color);paint.setAlpha(68*alpha/255);canvas.drawCircle(cx,cy,7.5f*unit,paint);
  paint.setAlpha(alpha);canvas.drawCircle(cx,cy,4.2f*unit,paint);
  paint.setColor(0xfff4fcff);paint.setAlpha(alpha);canvas.drawCircle(cx,cy,1.8f*unit,paint);
 }
 public void setAlpha(int alpha){this.alpha=alpha;invalidateSelf();}
 public void setColorFilter(ColorFilter filter){paint.setColorFilter(filter);invalidateSelf();}
 public int getOpacity(){return PixelFormat.TRANSLUCENT;}
}
