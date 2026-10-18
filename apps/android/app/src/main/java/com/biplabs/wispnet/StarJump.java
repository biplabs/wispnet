package com.biplabs.wispnet;
import android.animation.*;
import android.content.Context;
import android.graphics.*;
import android.view.View;
/** Local canvas starfield and 3-beat hyperjump effect (lock-on, warp surge, arrival shockwave). */
final class StarJump extends View {
 final Paint paint=new Paint(Paint.ANTI_ALIAS_FLAG);
 final float[] stars=new float[480];float progress=0;boolean hyperspace=false,decelerating=false;ValueAnimator animation;Shader glow,ambient,nebula,hyperGlow;
 StarJump(Context context){super(context);paint.setStrokeCap(Paint.Cap.ROUND);java.util.Random random=new java.util.Random(17);for(int i=0;i<stars.length;i++)stars[i]=random.nextFloat();setImportantForAccessibility(IMPORTANT_FOR_ACCESSIBILITY_NO);}
 void jump(Runnable done){
  cancel();if(!ValueAnimator.areAnimatorsEnabled()){done.run();return;}
  decelerating=false;animation=ValueAnimator.ofFloat(0,1);animation.setDuration(900);
  animation.setInterpolator(new android.view.animation.LinearInterpolator());
  animation.addUpdateListener(a->{progress=(float)a.getAnimatedValue();invalidate();});
  animation.addListener(new AnimatorListenerAdapter(){boolean cancelled;public void onAnimationCancel(Animator a){cancelled=true;}public void onAnimationEnd(Animator a){animation=null;progress=0;invalidate();if(!cancelled)done.run();}});animation.start();
 }
 void dropOut(){
  cancel();if(!ValueAnimator.areAnimatorsEnabled())return;
  decelerating=true;animation=ValueAnimator.ofFloat(1,0);animation.setDuration(320);
  animation.addUpdateListener(a->{progress=(float)a.getAnimatedValue()*.45f;invalidate();});
  animation.addListener(new AnimatorListenerAdapter(){public void onAnimationEnd(Animator a){animation=null;decelerating=false;progress=0;invalidate();}});animation.start();
 }
 void cancel(){if(animation!=null)animation.cancel();animation=null;decelerating=false;progress=0;invalidate();}
 protected void onSizeChanged(int w,int h,int oldw,int oldh){
  float r=Math.max(1,Math.min(w,h)*.48f);
  glow=new RadialGradient(w/2f,h/2f,r,new int[]{0xffe4fbff,0xff65ceff,0x663c60dc,0x00101938},new float[]{0,.045f,.35f,1},Shader.TileMode.CLAMP);
  ambient=new RadialGradient(w/2f,h*.45f,Math.max(1,Math.max(w,h)*.68f),new int[]{0x48264b8a,0x2414284c,0x00070b14},new float[]{0,.52f,1},Shader.TileMode.CLAMP);
  nebula=new RadialGradient(w*.28f,h*.24f,Math.max(1,Math.min(w,h)*.55f),new int[]{0x28386fd4,0x121c3b73,0x00070b14},new float[]{0,.5f,1},Shader.TileMode.CLAMP);
  hyperGlow=new RadialGradient(w/2f,h*.38f,Math.max(1,Math.max(w,h)*.75f),new int[]{0x5522569e,0x2c163870,0x00070b14},new float[]{0,.58f,1},Shader.TileMode.CLAMP);
 }
 protected void onDraw(Canvas canvas){
  super.onDraw(canvas);float d=getResources().getDisplayMetrics().density,cx=getWidth()/2f,cy=getHeight()/2f,scale=(float)Math.hypot(cx,cy);boolean warp=animation!=null;
  long now=android.os.SystemClock.elapsedRealtime();float speedMul=hyperspace?1.8f:1f,cruise=((now*(long)(speedMul*100))/2400000f)%1f,pulse=.82f+.18f*(float)Math.sin(now*.0016);
  Shader bg=hyperspace&&hyperGlow!=null?hyperGlow:ambient;
  if(bg!=null){paint.setShader(bg);paint.setAlpha((int)(255*pulse));canvas.drawRect(0,0,getWidth(),getHeight(),paint);paint.setShader(null);}
  if(nebula!=null){paint.setShader(nebula);paint.setAlpha(hyperspace?245:210);canvas.drawRect(0,0,getWidth(),getHeight(),paint);paint.setShader(null);}
  float lockOn=warp&&!decelerating&&progress<.18f?(float)Math.sin((progress/.18f)*Math.PI):0;
  float warpPhase=warp&&!decelerating?Math.max(0,(progress-.14f)/.86f):progress;
  float surge=warp?(float)Math.sin(Math.PI*Math.pow(warpPhase,.75)):0;
  if(warp&&glow!=null){paint.setShader(glow);paint.setAlpha((int)(255*surge));canvas.drawRect(0,0,getWidth(),getHeight(),paint);paint.setShader(null);}
  if(lockOn>0){paint.setStyle(Paint.Style.STROKE);paint.setStrokeWidth(1.5f*d);paint.setColor(0xff65ceff);paint.setAlpha((int)(140*lockOn));canvas.drawCircle(cx,cy,(68-lockOn*22)*d,paint);paint.setStyle(Paint.Style.FILL);}
  if(warp&&!decelerating&&progress>.76f){float wave=(progress-.76f)/.24f;paint.setStyle(Paint.Style.STROKE);paint.setStrokeWidth((3.2f-wave*2.2f)*d);paint.setColor(0xff65ceff);paint.setAlpha((int)((1-wave)*170));canvas.drawCircle(cx,cy,(.06f+wave*.78f)*scale,paint);paint.setStyle(Paint.Style.FILL);}
  int count=warp?stars.length:330;
  for(int i=0;i<count;i+=3){
   float angle=stars[i]*6.283185f,speed=.6f+stars[i+2]*.8f,depth=(stars[i+1]+cruise*speed+(warp?warpPhase*.75f:0))%1;
   float radius=Math.max(4*d,((.03f+depth*depth)-lockOn*.022f)*scale);
   float x=(float)Math.cos(angle),y=(float)Math.sin(angle),twinkle=.7f+.3f*(float)Math.sin(now*.004*(.5f+stars[i+2])+i);
   float tail=warp?surge*surge*(.025f+depth*.42f)*scale:(hyperspace?depth*depth*13*d:depth*depth*6*d);
   int color=i%15==0?0xffffdfa8:i%9==0?0xff7da2ff:i%6==0?0xff65ceff:0xfff0f8ff;
   int alpha=(int)((warp?120+stars[i+2]*135:(85+depth*155)*twinkle)*(warp?Math.min(1,.35f+surge*3):1));
   float width=(warp?1.3f+stars[i+2]*1.9f:1.1f+depth*2.1f*(.6f+stars[i+2]*.6f))*d;
   paint.setColor(color);
   if(!warp&&depth>.25f){paint.setAlpha(alpha/4);canvas.drawCircle(cx+x*radius,cy+y*radius,width*1.6f,paint);}
   paint.setAlpha(Math.min(255,alpha));paint.setStrokeWidth(width);
   if(tail<1.5f*d)canvas.drawCircle(cx+x*radius,cy+y*radius,width/2,paint);
   else canvas.drawLine(cx+x*radius,cy+y*radius,cx+x*(radius+tail),cy+y*(radius+tail),paint);
  }
  if(!warp&&isAttachedToWindow()&&getWindowVisibility()==VISIBLE&&ValueAnimator.areAnimatorsEnabled())postInvalidateDelayed(33);
 }
 protected void onDetachedFromWindow(){cancel();super.onDetachedFromWindow();}
}
