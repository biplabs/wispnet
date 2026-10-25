// SPDX-License-Identifier: Apache-2.0
package com.biplabs.wispnet.shell

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.zIndex
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import com.biplabs.wispnet.ShellRpc
import kotlinx.coroutines.delay
import org.json.JSONArray
import org.json.JSONObject

/** Native terminal surface; Flutter still owns device/service navigation. */
class ShellActivity : ComponentActivity() {
    private lateinit var rpc: ShellRpc

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val target = try { ShellTarget.parse(JSONObject(intent.getStringExtra("target") ?: "{}")) }
        catch (_: Exception) { finish(); return }
        val requestedSession = intent.getStringExtra("session")?.takeIf { it.matches(Regex("[a-z0-9._-]{1,64}")) } ?: "main"
        rpc = ShellRpc(this)
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowCompat.getInsetsController(window, window.decorView).apply {
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            hide(WindowInsetsCompat.Type.statusBars())
        }
        setContent {
            MaterialTheme(colorScheme = darkColorScheme()) {
                ShellScreen(target, requestedSession, rpc, onBack = ::finish)
            }
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        if (::rpc.isInitialized) rpc.close()
    }
}

private data class ShellTab(val target: ShellTarget, val session: String) {
    val id: String get() = "${target.host}/${target.service}/${target.incarnation}/$session"
    fun json(): JSONObject = JSONObject().put("target", target.json()).put("session", session)
}

@Composable
private fun ShellScreen(initial: ShellTarget, requestedSession: String, rpc: ShellRpc, onBack: () -> Unit) {
    val context = LocalContext.current
    val preferences = remember { context.getSharedPreferences("shell-terminal", Context.MODE_PRIVATE) }
    var tabs by remember {
        val restored = runCatching {
            val saved = JSONArray(preferences.getString("tabs", "[]"))
            (0 until minOf(saved.length(), 16)).mapNotNull { index ->
                runCatching {
                    val entry = saved.getJSONObject(index)
                    val name = entry.getString("session")
                    require(name.matches(Regex("[a-z0-9._-]{1,64}")))
                    ShellTab(ShellTarget.parse(entry.getJSONObject("target")), name)
                }.getOrNull()
            }
        }.getOrDefault(emptyList())
        val requested = ShellTab(initial, requestedSession)
        mutableStateOf((restored.filter { it.id != requested.id }.takeLast(15) + requested).distinctBy { it.id })
    }
    var selected by remember { mutableStateOf(ShellTab(initial, requestedSession).id) }
    var inputMode by remember {
        mutableStateOf(runCatching {
            TerminalInputMode.valueOf(preferences.getString("input-mode", "Predictive")!!)
        }.getOrDefault(TerminalInputMode.Predictive))
    }
    var modeMenu by remember { mutableStateOf(false) }
    val titles = remember { mutableStateMapOf<String, String>() }
    val paths = remember { mutableStateMapOf<String, TransportPathStatus>() }
    val states = remember { mutableStateMapOf<String, ConnectionState>() }
    val inputStates = remember { mutableStateMapOf<String, Pair<Boolean, Boolean>>() }
    val errors = remember { mutableStateMapOf<String, String>() }
    val senders = remember { mutableStateMapOf<String, (String) -> Unit>() }
    val retry = remember { mutableStateMapOf<String, Int>() }
    val lines = remember { mutableStateMapOf<String, String>() }
    var keyboardNonce by remember { mutableIntStateOf(0) }
    val networkNonce = rememberNetworkChanges()
    val current = tabs.firstOrNull { it.id == selected } ?: tabs.firstOrNull()
    val currentId = current?.id.orEmpty()
    fun modeFor(id: String?): TerminalInputMode {
        val state = inputStates[id.orEmpty()]
        // A command composer or local echo must not display password input.
        return inputMode.resolve(paths[id.orEmpty()]?.latencyMs, state?.first, state?.second)
    }
    val effectiveMode = modeFor(current?.id)

    LaunchedEffect(tabs) {
        preferences.edit().putString("tabs", JSONArray().apply { tabs.forEach { put(it.json()) } }.toString()).apply()
        if (tabs.none { it.id == selected }) selected = tabs.firstOrNull()?.id.orEmpty()
    }
    LaunchedEffect(inputMode) { preferences.edit().putString("input-mode", inputMode.name).apply() }

    Column(Modifier.fillMaxSize().background(Color.Black).navigationBarsPadding().imePadding()) {
        Row(Modifier.fillMaxWidth().height(44.dp), verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = onBack) { Text("Back") }
            Text(current?.target?.name ?: "Shell", Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
            Box {
                TextButton(onClick = { modeMenu = true }) {
                    Text(if (inputMode == TerminalInputMode.Auto) "Auto · ${effectiveMode.name}" else inputMode.name)
                }
                DropdownMenu(expanded = modeMenu, onDismissRequest = { modeMenu = false }) {
                    TerminalInputMode.entries.forEach { mode ->
                        DropdownMenuItem(text = { Text(mode.name) }, onClick = { inputMode = mode; modeMenu = false })
                    }
                }
            }
        }
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), verticalAlignment = Alignment.CenterVertically) {
            tabs.forEach { tab ->
                FilterChip(selected = tab.id == current?.id, onClick = { selected = tab.id },
                    label = { Text("${tab.target.name}: ${titles[tab.id] ?: tab.session}", maxLines = 1) },
                    modifier = Modifier.padding(horizontal = 3.dp))
            }
            TextButton(enabled = tabs.size < 16, onClick = {
                val target = current?.target ?: initial
                val tab = ShellTab(target, "tab-${System.currentTimeMillis()}")
                tabs = tabs + tab; selected = tab.id
            }) { Text("+") }
            TextButton(enabled = current != null, onClick = {
                tabs = tabs.filterNot { it.id == currentId }
                senders.remove(currentId); paths.remove(currentId); errors.remove(currentId); states.remove(currentId)
            }) { Text("Close tab") }
        }
        Row(Modifier.fillMaxWidth().padding(horizontal = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            val path = paths[currentId]
            val state = states[currentId] ?: ConnectionState.Connecting
            val status = when (state) {
                ConnectionState.Attached -> buildString {
                    append("Connected")
                    path?.let { append(" · ${it.path}"); it.latencyMs?.let { latency -> append(" · ${latency} ms") } }
                }
                ConnectionState.Connecting -> "Connecting…"
                ConnectionState.Disconnected -> errors[currentId] ?: "Disconnected"
                ConnectionState.Exited -> "Shell exited"
            }
            Text(status, Modifier.weight(1f), style = MaterialTheme.typography.labelSmall, maxLines = 2)
            if (state == ConnectionState.Disconnected || state == ConnectionState.Exited) {
                TextButton(onClick = { current?.let { retry[it.id] = (retry[it.id] ?: 0) + 1 } }) { Text("Reconnect") }
            }
        }
        Box(Modifier.weight(1f).fillMaxWidth()) {
            tabs.forEach { tab -> key(tab.id) {
                val active = tab.id == current?.id
                var reconnectAttempts by remember { mutableIntStateOf(0) }
                LaunchedEffect(networkNonce) { reconnectAttempts = 0 }
                LaunchedEffect(states[tab.id]) {
                    when (states[tab.id]) {
                        ConnectionState.Attached -> reconnectAttempts = 0
                        ConnectionState.Disconnected -> if (reconnectAttempts < 3) {
                            delay(1_000L shl reconnectAttempts)
                            reconnectAttempts++
                            retry[tab.id] = (retry[tab.id] ?: 0) + 1
                        }
                        else -> Unit
                    }
                }
                WispTermuxTerminalView(
                    modifier = Modifier.fillMaxSize().zIndex(if (active) 1f else 0f),
                    target = tab.target,
                    rpc = rpc,
                    sessionName = tab.session,
                    active = active,
                    reconnectNonce = (retry[tab.id] ?: 0) + networkNonce,
                    inputMode = modeFor(tab.id),
                    onConnectionState = { state ->
                        states[tab.id] = state
                        if (state == ConnectionState.Attached) errors.remove(tab.id)
                        else inputStates.remove(tab.id)
                    },
                    onConnectionError = { errors[tab.id] = it },
                    onTransportPathChanged = { path -> if (path == null) paths.remove(tab.id) else paths[tab.id] = path },
                    onTitleChanged = { titles[tab.id] = it },
                    onTerminalModeChanged = { echo, canonical -> inputStates[tab.id] = echo to canonical },
                    onSendInputReady = { sender -> if (sender == null) senders.remove(tab.id) else senders[tab.id] = sender },
                    onKeyboardRequest = { if (active) keyboardNonce++ },
                )
            } }
        }
        if (effectiveMode == TerminalInputMode.Line && current != null) {
            LineComposer(lines[current.id].orEmpty(), keyboardNonce,
                onChange = { lines[current.id] = it },
                onSend = {
                    senders[current.id]?.invoke(lines[current.id].orEmpty() + "\r")
                    lines[current.id] = ""
                })
        }
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.SpaceEvenly) {
            listOf("Ctrl-C" to "\u0003", "Esc" to "\u001b", "Tab" to "\t", "↑" to "\u001b[A",
                "↓" to "\u001b[B", "←" to "\u001b[D", "→" to "\u001b[C").forEach { (label, bytes) ->
                TextButton(enabled = states[currentId] == ConnectionState.Attached,
                    onClick = { senders[currentId]?.invoke(bytes) }, contentPadding = PaddingValues(8.dp)) { Text(label) }
            }
        }
    }
}

@Composable
private fun LineComposer(value: String, focusNonce: Int, onChange: (String) -> Unit, onSend: () -> Unit) {
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    LaunchedEffect(focusNonce) { if (focusNonce > 0) { focus.requestFocus(); keyboard?.show() } }
    Row(Modifier.fillMaxWidth().padding(horizontal = 10.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        BasicTextField(value = value, onValueChange = onChange, singleLine = true,
            textStyle = TextStyle(color = Color.White, fontFamily = FontFamily.Monospace, fontSize = 15.sp),
            cursorBrush = SolidColor(Color.White),
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
            keyboardActions = KeyboardActions(onSend = { onSend() }),
            modifier = Modifier.weight(1f).focusRequester(focus),
            decorationBox = { inner -> Box { if (value.isEmpty()) Text("Enter command…", color = Color.Gray); inner() } })
        TextButton(onClick = onSend) { Text("Send") }
    }
}

@Composable
private fun rememberNetworkChanges(): Int {
    val context = LocalContext.current
    var nonce by remember { mutableIntStateOf(0) }
    DisposableEffect(context) {
        val manager = context.getSystemService(ConnectivityManager::class.java)
        val main = Handler(Looper.getMainLooper())
        var previous = manager.activeNetwork
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                main.post {
                    if (network != previous) {
                        previous = network
                        // Give the shared networking process time to rebind its endpoint.
                        main.postDelayed({ nonce++ }, 750)
                    }
                }
            }
            override fun onLost(network: Network) { main.post { if (previous == network) previous = null } }
        }
        manager.registerDefaultNetworkCallback(callback)
        onDispose { manager.unregisterNetworkCallback(callback); main.removeCallbacksAndMessages(null) }
    }
    return nonce
}
