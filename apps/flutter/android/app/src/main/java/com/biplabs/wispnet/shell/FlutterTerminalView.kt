// SPDX-License-Identifier: AGPL-3.0-only
package com.biplabs.wispnet.shell

import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.view.MotionEvent
import android.view.View
import android.widget.FrameLayout
import com.termux.view.TerminalView
import io.flutter.plugin.common.BinaryMessenger
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.StandardMessageCodec
import io.flutter.plugin.platform.PlatformView
import io.flutter.plugin.platform.PlatformViewFactory
import org.json.JSONArray
import org.json.JSONObject

/** Flutter owns terminal chrome; the existing native engine owns VT and IME state. */
object FlutterTerminalBridge {
    internal val keyboardRequests = mutableMapOf<String, () -> Unit>()
    private fun tab(arguments: JSONObject): ShellTab {
        val session = arguments.getString("session")
        require(session.matches(Regex("[a-z0-9._-]{1,64}"))) { "Invalid shell session" }
        return ShellTab(ShellTarget.parse(arguments), session)
    }

    internal fun holder(context: Context, tab: ShellTab): TermuxTerminalHolder {
        ShellWorkspace.openRequest(context, tab.target, tab.session)
        return ShellWorkspace.holderFor(context, tab, TerminalInputMode.Predictive).also { holder ->
            holder.updateCallbacks(
                { ShellWorkspace.states[tab.id] = it },
                { ShellWorkspace.errors[tab.id] = it },
                { if (it == null) ShellWorkspace.paths.remove(tab.id) else ShellWorkspace.paths[tab.id] = it },
                { ShellWorkspace.titles[tab.id] = it },
                { echo, canonical -> ShellWorkspace.inputStates[tab.id] = echo to canonical },
                { keyboardRequests[tab.id]?.invoke() },
            )
        }
    }

    @JvmStatic
    fun dispatch(context: Context, method: String, arguments: JSONObject): JSONObject {
        if (method == "saveTerminalLayout") {
            val layout = arguments.getJSONObject("layout")
            require(layout.toString().length <= 131072) { "Terminal layout is too large" }
            context.applicationContext.getSharedPreferences("shell-terminal", Context.MODE_PRIVATE)
                .edit().putString("flutter_layout", layout.toString()).apply()
            return JSONObject()
        }
        val tab = tab(arguments)
        when (method) {
            "nativeTerminalOpen" -> holder(context, tab).apply { connectDetached(); reconnectIfDisconnected() }
            "nativeTerminalInput" -> holder(context, tab).sendInput(arguments.getString("data"))
            "nativeTerminalReconnect" -> holder(context, tab).reconnectExplicitly()
            "nativeTerminalClose" -> {
                ShellWorkspace.closeTab(context, tab.id, arguments.optBoolean("terminate", false))
                return JSONObject()
            }
            "nativeTerminalStatus" -> Unit
            else -> throw IllegalArgumentException("Unknown terminal operation")
        }
        val path = ShellWorkspace.paths[tab.id]
        val mode = ShellWorkspace.inputStates[tab.id]
        return JSONObject().put("state", (ShellWorkspace.states[tab.id] ?: ConnectionState.Connecting).name.lowercase())
            .put("path", path?.path ?: "unknown").put("latency_ms", path?.latencyMs)
            .put("echo_enabled", mode?.first == true).put("canonical", mode?.second == true)
            .put("title", ShellWorkspace.titles[tab.id]).put("error", ShellWorkspace.errors[tab.id])
    }

    @JvmStatic
    fun savedLayout(context: Context): JSONObject {
        val preferences = context.applicationContext.getSharedPreferences("shell-terminal", Context.MODE_PRIVATE)
        val saved = preferences.getString("flutter_layout", null)
        if (saved != null) return runCatching { JSONObject(saved) }.getOrDefault(JSONObject())
        // Upgrade the previous native page's persisted tabs without dropping PTYs.
        val tabs = runCatching { JSONArray(preferences.getString("tabs", "[]")) }.getOrDefault(JSONArray())
        val hosts = linkedMapOf<String, JSONObject>()
        for (index in 0 until minOf(tabs.length(), 256)) runCatching {
            val entry = tabs.getJSONObject(index)
            val target = ShellTarget.parse(entry.getJSONObject("target"))
            val session = entry.getString("session")
            require(session.matches(Regex("[a-z0-9._-]{1,64}")))
            val workspace = hosts.getOrPut(target.host) {
                target.json().put("host_name", target.name).put("minimized", true).put("tabs", JSONArray())
            }
            workspace.getJSONArray("tabs").put(JSONObject().put("id", session)
                .put("axis", "horizontal").put("active", session).put("panes", JSONArray().put(session)))
        }
        return JSONObject().put("mode", "predictive").put("workspaces", JSONArray(hosts.values.toList()))
    }
}

class FlutterTerminalViewFactory(private val messenger: BinaryMessenger) : PlatformViewFactory(StandardMessageCodec.INSTANCE) {
    override fun create(context: Context, viewId: Int, args: Any?): PlatformView {
        val values = args as Map<*, *>
        return FlutterTerminalView(context, viewId, JSONObject(values.entries.associate { it.key.toString() to it.value }), messenger)
    }
}

private class FlutterTerminalView(context: Context, viewId: Int, arguments: JSONObject, messenger: BinaryMessenger) : PlatformView {
    private val tab = ShellTab(ShellTarget.parse(arguments), arguments.getString("session"))
    private val holder = FlutterTerminalBridge.holder(context, tab)
    private val channel = MethodChannel(messenger, "app.wispnet/terminal/$viewId")
    private val keyboardRequest: () -> Unit = { channel.invokeMethod("keyboard", null) }
    private val terminal = TerminalView(context, null).apply {
        setBackgroundColor(Color.BLACK)
        setTextSize((14 * context.resources.displayMetrics.scaledDensity).toInt())
        setTypeface(Typeface.MONOSPACE)
        isFocusable = true
        isFocusableInTouchMode = true
    }
    private val input = TerminalInputEditText(context, holder::sendInputFromIme)
    private val container = object : FrameLayout(context) {
        override fun dispatchTouchEvent(event: MotionEvent): Boolean {
            if (event.actionMasked == MotionEvent.ACTION_DOWN) channel.invokeMethod("focus", null)
            return super.dispatchTouchEvent(event)
        }
    }.apply {
        addView(terminal, FrameLayout.LayoutParams(-1, -1))
        addView(input, FrameLayout.LayoutParams(1, 1))
    }
    init {
        FlutterTerminalBridge.keyboardRequests[tab.id] = keyboardRequest
        holder.attachView(terminal, input)
        channel.setMethodCallHandler { call, result ->
            if (call.method == "update") {
                update(JSONObject(call.arguments as Map<*, *>))
                result.success(null)
            } else result.notImplemented()
        }
        update(arguments)
    }
    private fun update(arguments: JSONObject) {
        val mode = when (arguments.optString("mode")) {
            "line" -> TerminalInputMode.Line
            "predictive" -> TerminalInputMode.Predictive
            "auto" -> TerminalInputMode.Auto
            else -> TerminalInputMode.Sync
        }
        holder.setActive(arguments.optBoolean("active", true))
        holder.setInputMode(mode)
    }
    override fun getView(): View = container
    override fun dispose() {
        if (FlutterTerminalBridge.keyboardRequests[tab.id] === keyboardRequest)
            FlutterTerminalBridge.keyboardRequests.remove(tab.id)
        channel.setMethodCallHandler(null)
        holder.detachView(terminal)
    }
}
