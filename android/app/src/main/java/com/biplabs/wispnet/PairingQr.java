package com.biplabs.wispnet;
import android.graphics.*;
import java.io.*;
import java.util.*;
import com.google.zxing.*;
import com.google.zxing.common.HybridBinarizer;

/** Bounded local image decoding; the result still needs native invitation review. */
final class PairingQr {
 static String decode(InputStream input)throws Exception{
  byte[] bytes=input.readNBytes(4*1024*1024+1);
  if(bytes.length>4*1024*1024)throw new IOException("QR image too large");
  BitmapFactory.Options options=new BitmapFactory.Options();options.inJustDecodeBounds=true;BitmapFactory.decodeByteArray(bytes,0,bytes.length,options);
  if(options.outWidth<=0||options.outHeight<=0||options.outWidth>16384||options.outHeight>16384)throw new IOException("Invalid QR image size");
  options.inJustDecodeBounds=false;options.inSampleSize=1;
  while(options.outWidth/options.inSampleSize>2048||options.outHeight/options.inSampleSize>2048)options.inSampleSize*=2;
  Bitmap bitmap=BitmapFactory.decodeByteArray(bytes,0,bytes.length,options);if(bitmap==null)throw new IOException("Invalid QR image");
  try{int w=bitmap.getWidth(),h=bitmap.getHeight();int[] pixels=new int[w*h];bitmap.getPixels(pixels,0,w,0,0,w,h);
   Map<DecodeHintType,Object> hints=new EnumMap<>(DecodeHintType.class);hints.put(DecodeHintType.POSSIBLE_FORMATS,Collections.singletonList(BarcodeFormat.QR_CODE));hints.put(DecodeHintType.TRY_HARDER,true);
   BinaryBitmap binary=new BinaryBitmap(new HybridBinarizer(new RGBLuminanceSource(w,h,pixels)));String text;
   try{text=new MultiFormatReader().decode(binary,hints).getText();}catch(NotFoundException notDetected){hints.put(DecodeHintType.PURE_BARCODE,true);text=new MultiFormatReader().decode(binary,hints).getText();}
   if(text.getBytes(java.nio.charset.StandardCharsets.UTF_8).length>8192)throw new IOException("QR payload too large");return text;
  }finally{bitmap.recycle();}
 }
}
