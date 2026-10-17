package com.biplabs.wispnet;
import android.animation.*;
import android.content.Context;
import android.graphics.*;
import android.view.View;
/** Local canvas warp effect; renders only during the bounded entry transition. */
final class StarJump extends View {
 final Paint paint=new Paint(Paint.ANTI_ALIAS_FLAG);
 final float[] stars=new float[2700];float progress=0;ValueAnimator animation;Shader glow,ambient;
 StarJump(Context context){super(context);java.util.Random random=new java.util.Random(17);for(int i=0;i<stars.length;i++)stars[i]=random.nextFloat();setImportantForAccessibility(IMPORTANT_FOR_ACCESSIBILITY_NO);}
 void jump(Runnable done){
  cancel();if(!ValueAnimator.areAnimatorsEnabled()){done.run();return;}
  animation=ValueAnimator.ofFloat(0,1);animation.setDuration(2200);
  animation.setInterpolator(new android.view.animation.LinearInterpolator());
  animation.addUpdateListener(a->{progress=(float)a.getAnimatedValue();invalidate();});
  animation.addListener(new AnimatorListenerAdapter(){boolean cancelled;public void onAnimationCancel(Animator a){cancelled=true;}public void onAnimationEnd(Animator a){animation=null;progress=0;invalidate();if(!cancelled)done.run();}});animation.start();
 }
 void cancel(){if(animation!=null)animation.cancel();animation=null;progress=0;invalidate();}
 protected void onSizeChanged(int w,int h,int oldw,int oldh){
  float r=Math.max(1,Math.min(w,h)*.48f);
  glow=new RadialGradient(w/2f,h/2f,r,new int[]{0xffe4fbff,0xff65ceff,0x663c60dc,0x00101938},new float[]{0,.045f,.35f,1},Shader.TileMode.CLAMP);
  ambient=new RadialGradient(w/2f,h*.42f,Math.max(1,Math.max(w,h)*.62f),new int[]{0x2e2a4b8d,0x1613254a,0x00070b14},new float[]{0,.48f,1},Shader.TileMode.CLAMP);
 }
 protected void onDraw(Canvas canvas){
  super.onDraw(canvas);float cx=getWidth()/2f,cy=getHeight()/2f,scale=(float)Math.hypot(cx,cy);boolean warp=animation!=null;
  if(ambient!=null){paint.setShader(ambient);paint.setAlpha(255);canvas.drawRect(0,0,getWidth(),getHeight(),paint);paint.setShader(null);}
  float surge=warp?(float)Math.sin(Math.PI*Math.pow(progress,.75)):0;
  if(warp&&glow!=null){paint.setShader(glow);paint.setAlpha((int)(255*surge));canvas.drawRect(0,0,getWidth(),getHeight(),paint);paint.setShader(null);}
  int count=warp?stars.length:420;
  for(int i=0;i<count;i+=3){float angle=stars[i]*6.283185f,depth=(stars[i+1]+progress*.85f)%1,radius=(.015f+depth*depth)*scale;
   float tail=surge*surge*(.03f+depth*.65f)*scale,x=(float)Math.cos(angle),y=(float)Math.sin(angle);
   paint.setColor(i%9==0?0xff7da2ff:i%6==0?0xff65ceff:0xffe2f6ff);paint.setAlpha((int)((warp?110+stars[i+2]*145:42+stars[i+2]*85)*(warp?Math.min(1,surge*4):1)));paint.setStrokeWidth((warp?1.2f:1)+stars[i+2]*(warp?2.1f:1.2f));
   if(tail<1)canvas.drawCircle(cx+x*radius,cy+y*radius,paint.getStrokeWidth()/2,paint);else canvas.drawLine(cx+x*radius,cy+y*radius,cx+x*(radius+tail),cy+y*(radius+tail),paint);
  }
 }
 protected void onDetachedFromWindow(){cancel();super.onDetachedFromWindow();}
}
