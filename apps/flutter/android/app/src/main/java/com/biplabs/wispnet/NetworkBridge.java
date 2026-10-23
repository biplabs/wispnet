package com.biplabs.wispnet;

import android.content.*;
import android.os.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicInteger;
import org.json.JSONObject;

/** Private Binder RPC. The UI never initializes JNI or reads network state files. */
final class NetworkBridge implements AutoCloseable {
 static final class Failure extends Exception {
  final String code;
  Failure(String code, String message) { super(message); this.code = code; }
 }
 private final Context context;
 private volatile CompletableFuture<Messenger> connected = new CompletableFuture<>();
 private final ConcurrentHashMap<Integer, CompletableFuture<JSONObject>> pending = new ConcurrentHashMap<>();
 private final AtomicInteger sequence = new AtomicInteger();
 private volatile boolean closed;
 private final Messenger replies = new Messenger(new Handler(Looper.getMainLooper(), message -> {
  CompletableFuture<JSONObject> result = pending.remove(message.arg1);
  if (result != null) {
   Bundle data = message.getData();
   try {
    if (data.containsKey("error")) result.completeExceptionally(new Failure(data.getString("code", "operation_failed"), data.getString("error")));
    else result.complete(new JSONObject(data.getString("result", "{}")));
   } catch (Exception invalid) { result.completeExceptionally(invalid); }
  }
  return true;
 }));
 private final ServiceConnection connection = new ServiceConnection() {
  public void onServiceConnected(ComponentName name, IBinder binder) { connected.complete(new Messenger(binder)); }
  public void onServiceDisconnected(ComponentName name) {
   connected = new CompletableFuture<>();
   failPending(new Failure("network_stopped", "Networking restarted. Retry the operation."));
  }
  public void onBindingDied(ComponentName name) {
   onServiceDisconnected(name);
   if (!closed) {
    context.unbindService(this);
    if (!context.bindService(new Intent(context, AccessService.class), this, Context.BIND_AUTO_CREATE))
     connected.completeExceptionally(new Failure("network_unavailable", "Cannot reconnect networking service"));
   }
  }
  public void onNullBinding(ComponentName name) { onServiceDisconnected(name); }
 };
 NetworkBridge(Context context) {
  this.context = context.getApplicationContext();
  if (!this.context.bindService(new Intent(context, AccessService.class), connection, Context.BIND_AUTO_CREATE))
   connected.completeExceptionally(new Failure("network_unavailable", "Cannot start networking service"));
 }
 JSONObject request(String method, JSONObject arguments) throws Exception {
  if (closed) throw new Failure("activity_closed", "Window closed");
  int id = sequence.incrementAndGet();
  CompletableFuture<JSONObject> result = new CompletableFuture<>(); pending.put(id, result);
  try {
   Message message = Message.obtain(null, 1, id, 0); message.replyTo = replies;
   Bundle data = new Bundle(); data.putString("method", method); data.putString("arguments", arguments.toString()); message.setData(data);
   connected.get(15, TimeUnit.SECONDS).send(message);
   return result.get(90, TimeUnit.SECONDS);
  } catch (ExecutionException failure) {
   if (failure.getCause() instanceof Exception) throw (Exception) failure.getCause();
   throw failure;
  } finally { pending.remove(id); }
 }
 private void failPending(Exception failure) {
  for (CompletableFuture<JSONObject> result : pending.values()) result.completeExceptionally(failure);
  pending.clear();
 }
 public void close() {
  closed = true;
  Failure failure = new Failure("activity_closed", "Window closed");
  connected.completeExceptionally(failure); failPending(failure);
  try { context.unbindService(connection); } catch (IllegalArgumentException ignored) {}
 }
}
