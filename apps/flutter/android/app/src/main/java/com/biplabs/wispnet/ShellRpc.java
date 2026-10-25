// SPDX-License-Identifier: Apache-2.0
package com.biplabs.wispnet;

import android.content.Context;
import org.json.JSONObject;

/** Terminal adapter to the same private network process used by Flutter. */
public final class ShellRpc implements AutoCloseable {
 private final NetworkBridge bridge;
 private final java.util.Set<String> sessions = java.util.concurrent.ConcurrentHashMap.newKeySet();
 private final java.util.concurrent.atomic.AtomicInteger inFlight = new java.util.concurrent.atomic.AtomicInteger();
 private volatile boolean closing;
 public ShellRpc(Context context) { bridge = new NetworkBridge(context); }
 public JSONObject request(String operation, JSONObject arguments) throws Exception {
  if (!operation.startsWith("shell_")) throw new IllegalArgumentException("Invalid shell operation");
  if (closing && !operation.equals("shell_close")) throw new IllegalStateException("Terminal closed");
  inFlight.incrementAndGet();
  try {
   JSONObject result = bridge.request(operation, arguments);
   if (operation.equals("shell_open")) sessions.add(result.getString("session"));
   if (operation.equals("shell_close")) sessions.remove(arguments.getString("session"));
   return result;
  } finally { inFlight.decrementAndGet(); }
 }
 @Override public void close() {
  if (closing) return;
  closing = true;
  // Binder replies need the main looper, so never wait for detachment in an
  // Activity lifecycle callback. Host PTYs intentionally survive detachment.
  new Thread(() -> {
   // Let an outstanding shell_open return its handle before detaching it.
   while (inFlight.get() > 0) try { Thread.sleep(10); } catch (InterruptedException ignored) {}
   for (String session : sessions) try { bridge.request("shell_close", new JSONObject().put("session", session)); } catch (Exception ignored) {}
   sessions.clear(); bridge.close();
  }, "wispnet-shell-close").start();
 }
}
