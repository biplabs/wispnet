// SPDX-License-Identifier: Apache-2.0
package com.biplabs.wispnet.shell

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.Network
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.FastOutLinearInEasing
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.zIndex
import androidx.core.view.WindowCompat
import com.biplabs.wispnet.ShellRpc
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.roundToInt

private val SheetColor = Color(0xFF0B1324)
private val PanelColor = Color(0xFF101A30)
private val ButtonSurfaceColor = Color(0xFF142038)
private val SelectedTabColor = Color(0xFF1F3666)
private val BorderColor = Color(0xFF294270)
private val SelectedBorderColor = Color(0xFF4B73C2)
private val TextColor = Color(0xFFEDF2FF)
private val MutedColor = Color(0xFF94A6C4)
private val CyanColor = Color(0xFF65CEFF)
private val GreenColor = Color(0xFF7BE0B8)
private val AmberColor = Color(0xFFF0B85C)
private val ErrorColor = Color(0xFFFF9AA8)

internal data class ShellTab(val target: ShellTarget, val session: String) {
    val id: String get() = "${target.host}/${target.service}/${target.incarnation}/$session"
    fun json(): JSONObject = JSONObject().put("target", target.json()).put("session", session)
}

/** Keeps terminal tabs and PTY streams alive while the sheet is minimized. */
object ShellWorkspace {
    private var rpc: ShellRpc? = null
    private val holders = mutableMapOf<String, TermuxTerminalHolder>()
    private var networkManager: ConnectivityManager? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private val networkHandler by lazy { Handler(Looper.getMainLooper()) }
    internal var tabs by mutableStateOf<List<ShellTab>>(emptyList())
    internal var selected by mutableStateOf("")
    internal var minimized by mutableStateOf(false)
    internal val titles = mutableStateMapOf<String, String>()
    internal val paths = mutableStateMapOf<String, TransportPathStatus>()
    internal val states = mutableStateMapOf<String, ConnectionState>()
    internal val inputStates = mutableStateMapOf<String, Pair<Boolean, Boolean>>()
    internal val errors = mutableStateMapOf<String, String>()
    internal val senders = mutableStateMapOf<String, (String) -> Unit>()
    internal val lines = mutableStateMapOf<String, String>()

    @Synchronized
    internal fun ensureRpc(context: Context): ShellRpc {
        if (networkCallback == null) {
            val manager = context.applicationContext.getSystemService(ConnectivityManager::class.java)
            val callback = object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) {
                    // Also works while the sheet is minimized. A healthy stream is left alone.
                    networkHandler.post { holders.values.toList().forEach { it.reconnectIfDisconnected() } }
                }
            }
            manager.registerDefaultNetworkCallback(callback)
            networkManager = manager
            networkCallback = callback
        }
        return rpc ?: ShellRpc(context.applicationContext).also { rpc = it }
    }

    @Synchronized
    fun openRequest(context: Context, initial: ShellTarget, requestedSession: String) {
        ensureRpc(context)
        val preferences = context.applicationContext.getSharedPreferences("shell-terminal", Context.MODE_PRIVATE)
        val existing = if (tabs.isNotEmpty()) tabs else runCatching {
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
        val merged = if (existing.any { it.id == requested.id }) {
            existing
        } else {
            (existing.takeLast(15) + requested).distinctBy { it.id }
        }
        tabs = merged
        selected = requested.id
        minimized = false
        saveTabs(preferences, merged)
    }

    @Synchronized
    internal fun holderFor(
        context: Context,
        tab: ShellTab,
        inputMode: TerminalInputMode,
    ): TermuxTerminalHolder {
        val current = holders[tab.id]
        if (current != null) return current
        val holder = TermuxTerminalHolder(
            target = tab.target,
            rpc = ensureRpc(context),
            sessionName = tab.session,
            inputMode = inputMode,
            onConnectionState = {},
            onConnectionError = {},
            onTransportPathChanged = {},
            onTitleChanged = {},
            onTerminalModeChanged = { _, _ -> },
            onKeyboardRequest = {},
        )
        holders[tab.id] = holder
        return holder
    }

    @Synchronized
    internal fun addTab(context: Context, target: ShellTarget) {
        if (tabs.size >= 16) return
        val tab = ShellTab(target, "tab-${System.currentTimeMillis()}")
        val next = tabs + tab
        tabs = next
        selected = tab.id
        saveTabs(context.applicationContext.getSharedPreferences("shell-terminal", Context.MODE_PRIVATE), next)
    }

    @Synchronized
    internal fun closeTab(context: Context, id: String): Boolean {
        val next = tabs.filterNot { it.id == id }
        holders.remove(id)?.close()
        senders.remove(id)
        paths.remove(id)
        errors.remove(id)
        states.remove(id)
        inputStates.remove(id)
        titles.remove(id)
        lines.remove(id)
        if (next.isEmpty()) {
            closeAll(context)
            return true
        }
        tabs = next
        if ( next.none { it.id == selected } ) {
            selected = next.first().id
        }
        saveTabs(context.applicationContext.getSharedPreferences("shell-terminal", Context.MODE_PRIVATE), next)
        return false
    }

    @Synchronized
    fun minimize() {
        if (tabs.isNotEmpty()) {
            minimized = true
        }
    }

    @Synchronized
    fun closeHost(context: Context, host: String) {
        if (host.isEmpty()) return
        val matching = tabs.filter { it.target.host == host }.map { it.id }
        matching.forEach { closeTab(context, it) }
    }

    @Synchronized
    fun closeAll(context: Context? = null) {
        val activeHolders = holders.values.toList()
        holders.clear()
        activeHolders.forEach { it.close() }
        networkCallback?.let {
            networkManager?.unregisterNetworkCallback(it)
            networkHandler.removeCallbacksAndMessages(null)
        }
        networkCallback = null
        networkManager = null
        val activeRpc = rpc
        rpc = null
        activeRpc?.close()
        tabs = emptyList()
        selected = ""
        minimized = false
        titles.clear()
        paths.clear()
        states.clear()
        inputStates.clear()
        errors.clear()
        senders.clear()
        lines.clear()
        context?.applicationContext
            ?.getSharedPreferences("shell-terminal", Context.MODE_PRIVATE)
            ?.edit()
            ?.putString("tabs", "[]")
            ?.apply()
    }

    @Synchronized
    fun snapshotJson(): JSONObject? {
        if (tabs.isEmpty()) return null
        val current = tabs.firstOrNull { it.id == selected } ?: tabs.first()
        val suffix = titles[current.id] ?: current.session
        val countBadge = if (tabs.size > 1) " (${tabs.size})" else ""
        val title = "${current.target.name} · $suffix$countBadge"
        val stateName = when (states[current.id] ?: ConnectionState.Connecting) {
            ConnectionState.Attached -> "attached"
            ConnectionState.Connecting -> "connecting"
            ConnectionState.Disconnected -> "disconnected"
            ConnectionState.Exited -> "exited"
            ConnectionState.Failed -> "failed"
        }
        return JSONObject()
            .put("host", current.target.host)
            .put("host_name", current.target.name)
            .put("service", current.target.service)
            .put("incarnation", current.target.incarnation)
            .put("session", current.session)
            .put("title", title)
            .put("state", stateName)
            .put("tab_count", tabs.size)
            .put("minimized", minimized)
    }

    private fun saveTabs(preferences: android.content.SharedPreferences, currentTabs: List<ShellTab>) {
        preferences.edit().putString(
            "tabs",
            JSONArray().apply { currentTabs.forEach { put(it.json()) } }.toString(),
        ).apply()
    }
}

/** Native terminal sheet; Flutter owns device/service navigation and the minimized dock. */
class ShellActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        overrideActivityTransition(Activity.OVERRIDE_TRANSITION_OPEN, 0, 0)
        overrideActivityTransition(Activity.OVERRIDE_TRANSITION_CLOSE, 0, 0)
        if (!applyIntent(intent)) {
            finish()
            return
        }
        WindowCompat.setDecorFitsSystemWindows(window, false)
        window.addFlags(WindowManager.LayoutParams.FLAG_BLUR_BEHIND)
        setContent {
            MaterialTheme(
                colorScheme = darkColorScheme(
                    primary = CyanColor,
                    surface = SheetColor,
                    background = Color.Transparent,
                    error = ErrorColor,
                ),
            ) {
                ShellScreen(
                    onBlurProgress = ::updateWindowBlur,
                    onFinish = {
                        finish()
                        overrideActivityTransition(Activity.OVERRIDE_TRANSITION_CLOSE, 0, 0)
                    },
                )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        applyIntent(intent)
    }

    private fun applyIntent(intent: Intent?): Boolean {
        val target = try {
            ShellTarget.parse(JSONObject(intent?.getStringExtra("target") ?: "{}"))
        } catch (_: Exception) {
            return ShellWorkspace.tabs.isNotEmpty()
        }
        val requestedSession = intent?.getStringExtra("session")
            ?.takeIf { it.matches(Regex("[a-z0-9._-]{1,64}")) } ?: "main"
        ShellWorkspace.openRequest(this, target, requestedSession)
        return true
    }

    private fun updateWindowBlur(progress: Float) {
        val radius = (20f * progress.coerceIn(0f, 1f)).roundToInt()
        runCatching {
            window.setBackgroundBlurRadius(radius)
            val attrs = window.attributes
            if (attrs.blurBehindRadius != radius) {
                attrs.blurBehindRadius = radius
                window.attributes = attrs
            }
        }
    }
}

@Composable
private fun ShellScreen(
    onBlurProgress: (Float) -> Unit,
    onFinish: () -> Unit,
) {
    val context = LocalContext.current
    val density = LocalDensity.current
    val scope = rememberCoroutineScope()
    val focusManager = LocalFocusManager.current
    val keyboardController = LocalSoftwareKeyboardController.current
    val preferences = remember { context.getSharedPreferences("shell-terminal", Context.MODE_PRIVATE) }
    var inputMode by remember {
        mutableStateOf(
            runCatching {
                TerminalInputMode.valueOf(preferences.getString("input-mode", "Predictive")!!)
            }.getOrDefault(TerminalInputMode.Predictive),
        )
    }
    var modeMenu by remember { mutableStateOf(false) }
    var keyboardNonce by remember { mutableIntStateOf(0) }

    val sheetProgress = remember { Animatable(0f) }
    var dragOffsetPx by remember { mutableFloatStateOf(0f) }
    var sheetHeightPx by remember { mutableFloatStateOf(1600f) }
    var sheetSettled by remember { mutableStateOf(false) }
    var dismissing by remember { mutableStateOf(false) }

    val tabs = ShellWorkspace.tabs
    val selected = ShellWorkspace.selected
    val titles = ShellWorkspace.titles
    val paths = ShellWorkspace.paths
    val states = ShellWorkspace.states
    val inputStates = ShellWorkspace.inputStates
    val errors = ShellWorkspace.errors
    val senders = ShellWorkspace.senders
    val lines = ShellWorkspace.lines

    val current = tabs.firstOrNull { it.id == selected } ?: tabs.firstOrNull()
    val currentId = current?.id.orEmpty()

    fun modeFor(id: String?): TerminalInputMode {
        val state = inputStates[id.orEmpty()]
        // A command composer or local echo must not display password input.
        return inputMode.resolve(paths[id.orEmpty()]?.latencyMs, state?.first, state?.second)
    }
    val effectiveMode = modeFor(current?.id)

    val visualProgress = (sheetProgress.value * (1f - (dragOffsetPx / sheetHeightPx.coerceAtLeast(1f)).coerceIn(0f, 1f)))
        .coerceIn(0f, 1f)
    SideEffect {
        onBlurProgress(visualProgress)
    }

    fun dismiss(closeAll: Boolean) {
        if (dismissing) return
        dismissing = true
        sheetSettled = false
        modeMenu = false
        focusManager.clearFocus()
        keyboardController?.hide()
        if (closeAll) {
            ShellWorkspace.minimized = false
        } else {
            ShellWorkspace.minimize()
        }
        scope.launch {
            sheetProgress.animateTo(
                targetValue = 0f,
                animationSpec = tween(durationMillis = 200, easing = FastOutLinearInEasing),
            )
            onBlurProgress(0f)
            if (closeAll) {
                ShellWorkspace.closeAll(context)
            }
            onFinish()
        }
    }

    BackHandler(enabled = !dismissing) {
        if (modeMenu) {
            modeMenu = false
        } else {
            dismiss(closeAll = false)
        }
    }

    LaunchedEffect(Unit) {
        sheetProgress.animateTo(
            targetValue = 1f,
            animationSpec = tween(durationMillis = 250, easing = FastOutSlowInEasing),
        )
        sheetSettled = true
    }

    LaunchedEffect(inputMode) {
        preferences.edit().putString("input-mode", inputMode.name).apply()
    }

    val dismissThresholdPx = with(density) { 96.dp.toPx() }
    val slideOffsetPx = ((1f - sheetProgress.value) * sheetHeightPx + dragOffsetPx).roundToInt()

    Box(Modifier.fillMaxSize()) {
        // Blurred backdrop scrim; tapping above the sheet minimizes the terminal.
        Box(
            Modifier
                .fillMaxSize()
                .background(Color(0xFF040812).copy(alpha = 0.52f * visualProgress))
                .clickable(
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null,
                ) {
                    dismiss(closeAll = false)
                },
        )

        val sheetShape = RoundedCornerShape(topStart = 20.dp, topEnd = 20.dp)
        Column(
            Modifier
                .align(Alignment.BottomCenter)
                .fillMaxSize()
                .statusBarsPadding()
                .padding(top = 6.dp)
                .onSizeChanged { size ->
                    if (size.height > 0) {
                        sheetHeightPx = size.height.toFloat()
                    }
                }
                .offset { IntOffset(0, slideOffsetPx.coerceAtLeast(0)) }
                .clip(sheetShape)
                .background(SheetColor, sheetShape)
                .border(BorderStroke(1.dp, BorderColor), sheetShape)
                .clickable(
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null,
                ) {}
                .navigationBarsPadding()
                .imePadding(),
        ) {
            // Very top header bar (supports swipe-down to minimize)
            Column(
                Modifier
                    .fillMaxWidth()
                    .pointerInput(dismissing) {
                        if (dismissing) return@pointerInput
                        detectVerticalDragGestures(
                            onVerticalDrag = { change, dragAmount ->
                                change.consume()
                                dragOffsetPx = (dragOffsetPx + dragAmount).coerceAtLeast(0f)
                            },
                            onDragCancel = {
                                scope.launch { dragOffsetPx = 0f }
                            },
                            onDragEnd = {
                                if (dragOffsetPx >= dismissThresholdPx) {
                                    dismiss(closeAll = false)
                                } else {
                                    dragOffsetPx = 0f
                                }
                            },
                        )
                    }
                    .padding(fromTop = 5.dp, start = 12.dp, end = 10.dp, bottom = 2.dp),
            ) {
                Box(
                    Modifier
                        .align(Alignment.CenterHorizontally)
                        .padding(bottom = 3.dp)
                        .size(width = 32.dp, height = 3.dp)
                        .clip(RoundedCornerShape(1.5.dp))
                        .background(MutedColor.copy(alpha = 0.35f)),
                )

                val path = paths[currentId]
                val state = states[currentId] ?: ConnectionState.Connecting
                val statusText = when (state) {
                    ConnectionState.Attached -> path?.latencyMs?.let { "$it ms" }.orEmpty()
                    ConnectionState.Connecting -> "Connecting…"
                    ConnectionState.Disconnected -> "Disconnected · retrying…"
                    ConnectionState.Failed -> errors[currentId] ?: "Connection needs attention"
                    ConnectionState.Exited -> "Shell exited"
                }
                val dotColor = when (state) {
                    ConnectionState.Attached -> GreenColor
                    ConnectionState.Connecting -> AmberColor
                    ConnectionState.Disconnected, ConnectionState.Exited, ConnectionState.Failed -> ErrorColor
                }

                Row(
                    Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    SheetStatusDot(dotColor)
                    Spacer(Modifier.width(8.dp))
                    Text(
                        text = current?.target?.name ?: "Shell",
                        color = TextColor,
                        fontSize = 15.sp,
                        fontWeight = FontWeight.Bold,
                        maxLines = 1,
                    )
                    Text(
                        text = if (statusText.isNotEmpty()) " · $statusText" else "",
                        color = MutedColor,
                        fontSize = 12.sp,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    if (state == ConnectionState.Disconnected || state == ConnectionState.Exited || state == ConnectionState.Failed) {
                        Spacer(Modifier.width(6.dp))
                        Text(
                            text = if (state == ConnectionState.Exited) "Restart" else "Reconnect",
                            color = CyanColor,
                            fontSize = 12.sp,
                            fontWeight = FontWeight.Bold,
                            modifier = Modifier
                                .clip(RoundedCornerShape(6.dp))
                                .clickable {
                                    current?.let { ShellWorkspace.holderFor(context, it, modeFor(it.id)).reconnectExplicitly() }
                                }
                                .padding(horizontal = 4.dp, vertical = 2.dp),
                        )
                    }
                    Spacer(Modifier.width(8.dp))
                    val modeShape = RoundedCornerShape(14.dp)
                    Row(
                        Modifier
                            .height(28.dp)
                            .clip(modeShape)
                            .background(if (modeMenu) SelectedTabColor else ButtonSurfaceColor, modeShape)
                            .border(
                                BorderStroke(1.dp, if (modeMenu) SelectedBorderColor else BorderColor),
                                modeShape,
                            )
                            .semantics {
                                contentDescription = "Input mode"
                                role = Role.Button
                            }
                            .clickable { modeMenu = !modeMenu }
                            .padding(horizontal = 10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            text = if (inputMode == TerminalInputMode.Auto) {
                                "Auto · ${effectiveMode.name}"
                            } else {
                                inputMode.name
                            },
                            color = TextColor,
                            fontSize = 12.sp,
                            fontWeight = FontWeight.Medium,
                        )
                    }
                    Spacer(Modifier.width(6.dp))
                    HeaderIconButton(
                        description = "Minimize",
                        onClick = { dismiss(closeAll = false) },
                    ) {
                        Canvas(Modifier.size(12.dp)) {
                            val y = size.height * 0.55f
                            drawLine(
                                color = TextColor,
                                start = Offset(1.5f, y),
                                end = Offset(size.width - 1.5f, y),
                                strokeWidth = 1.8.dp.toPx(),
                                cap = StrokeCap.Round,
                            )
                        }
                    }
                    Spacer(Modifier.width(6.dp))
                    HeaderIconButton(
                        description = "Close",
                        onClick = { dismiss(closeAll = true) },
                    ) {
                        Canvas(Modifier.size(12.dp)) {
                            val pad = 1.5.dp.toPx()
                            drawLine(
                                color = TextColor,
                                start = Offset(pad, pad),
                                end = Offset(size.width - pad, size.height - pad),
                                strokeWidth = 1.8.dp.toPx(),
                                cap = StrokeCap.Round,
                            )
                            drawLine(
                                color = TextColor,
                                start = Offset(size.width - pad, pad),
                                end = Offset(pad, size.height - pad),
                                strokeWidth = 1.8.dp.toPx(),
                                cap = StrokeCap.Round,
                            )
                        }
                    }
                }
            }

            // Second line: Tabs (each with inline x) + "+" button
            val multipleHosts = remember(tabs) { tabs.map { it.target.host }.distinct().size > 1 }
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 10.dp, vertical = 2.dp)
                    .horizontalScroll(rememberScrollState()),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                tabs.forEach { tab ->
                    val isSelected = tab.id == current?.id
                    val tabTitle = titles[tab.id] ?: tab.session
                    val label = if (multipleHosts) "${tab.target.name}: $tabTitle" else tabTitle
                    val chipShape = RoundedCornerShape(8.dp)
                    Row(
                        Modifier
                            .padding(end = 6.dp)
                            .height(28.dp)
                            .clip(chipShape)
                            .background(if (isSelected) SelectedTabColor else PanelColor, chipShape)
                            .border(
                                BorderStroke(1.dp, if (isSelected) SelectedBorderColor else BorderColor),
                                chipShape,
                            )
                            .clickable { ShellWorkspace.selected = tab.id }
                            .padding(start = 10.dp, end = 6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            text = label,
                            color = if (isSelected) TextColor else MutedColor,
                            fontSize = 12.sp,
                            fontWeight = if (isSelected) FontWeight.SemiBold else FontWeight.Normal,
                            maxLines = 1,
                        )
                        Spacer(Modifier.width(6.dp))
                        Box(
                            Modifier
                                .size(18.dp)
                                .clip(CircleShape)
                                .semantics {
                                    contentDescription = "Close tab $label"
                                    role = Role.Button
                                }
                                .clickable {
                                    if (ShellWorkspace.closeTab(context, tab.id)) {
                                        dismiss(closeAll = true)
                                    }
                                },
                            contentAlignment = Alignment.Center,
                        ) {
                            Canvas(Modifier.size(8.dp)) {
                                val stroke = 1.5.dp.toPx()
                                val color = if (isSelected) TextColor else MutedColor
                                drawLine(
                                    color = color,
                                    start = Offset(0f, 0f),
                                    end = Offset(size.width, size.height),
                                    strokeWidth = stroke,
                                    cap = StrokeCap.Round,
                                )
                                drawLine(
                                    color = color,
                                    start = Offset(size.width, 0f),
                                    end = Offset(0f, size.height),
                                    strokeWidth = stroke,
                                    cap = StrokeCap.Round,
                                )
                            }
                        }
                    }
                }
                if (tabs.size < 16 && current != null) {
                    Box(
                        Modifier
                            .size(28.dp)
                            .clip(RoundedCornerShape(8.dp))
                            .background(PanelColor, RoundedCornerShape(8.dp))
                            .border(BorderStroke(1.dp, BorderColor), RoundedCornerShape(8.dp))
                            .semantics {
                                contentDescription = "New tab"
                                role = Role.Button
                            }
                            .clickable {
                                ShellWorkspace.addTab(context, current.target)
                            },
                        contentAlignment = Alignment.Center,
                    ) {
                        Text("+", color = CyanColor, fontSize = 15.sp, fontWeight = FontWeight.Bold)
                    }
                }
            }

            // Terminal viewport
            Box(
                Modifier
                    .weight(1f)
                    .fillMaxWidth()
                    .padding(horizontal = 8.dp, vertical = 4.dp)
                    .clip(RoundedCornerShape(14.dp))
                    .background(Color.Black)
                    .border(BorderStroke(1.dp, Color(0xFF1A2947)), RoundedCornerShape(14.dp)),
            ) {
                tabs.forEach { tab ->
                    key(tab.id) {
                        val active = tab.id == current?.id
                        val tabMode = modeFor(tab.id)
                        val holder = remember(tab.id) {
                            ShellWorkspace.holderFor(context, tab, tabMode)
                        }
                        WispTermuxTerminalView(
                            modifier = Modifier
                                .fillMaxSize()
                                .padding(horizontal = 4.dp, vertical = 2.dp)
                                .zIndex(if (active) 1f else 0f),
                            target = tab.target,
                            rpc = ShellWorkspace.ensureRpc(context),
                            existingHolder = holder,
                            retainOnDispose = true,
                            sessionName = tab.session,
                            active = active && sheetSettled && !dismissing,
                            inputMode = tabMode,
                            onConnectionState = { state ->
                                states[tab.id] = state
                                if (state == ConnectionState.Attached) errors.remove(tab.id)
                                else inputStates.remove(tab.id)
                            },
                            onConnectionError = { errors[tab.id] = it },
                            onTransportPathChanged = { path ->
                                if (path == null) paths.remove(tab.id) else paths[tab.id] = path
                            },
                            onTitleChanged = { titles[tab.id] = it },
                            onTerminalModeChanged = { echo, canonical ->
                                inputStates[tab.id] = echo to canonical
                            },
                            onSendInputReady = { sender ->
                                if (sender == null) senders.remove(tab.id) else senders[tab.id] = sender
                            },
                            onKeyboardRequest = { if (active) keyboardNonce++ },
                        )
                    }
                }
            }

            if (effectiveMode == TerminalInputMode.Line && current != null) {
                LineComposer(
                    value = lines[current.id].orEmpty(),
                    focusNonce = keyboardNonce,
                    onChange = { lines[current.id] = it },
                    onSend = {
                        senders[current.id]?.invoke(lines[current.id].orEmpty() + "\r")
                        lines[current.id] = ""
                    },
                )
            }

            // Shortcut keys bar
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 6.dp, vertical = 2.dp)
                    .horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.SpaceEvenly,
            ) {
                listOf(
                    "Ctrl-C" to "\u0003",
                    "Esc" to "\u001b",
                    "Tab" to "\t",
                    "↑" to "\u001b[A",
                    "↓" to "\u001b[B",
                    "←" to "\u001b[D",
                    "→" to "\u001b[C",
                ).forEach { (label, bytes) ->
                    TextButton(
                        enabled = states[currentId] == ConnectionState.Attached,
                        onClick = { senders[currentId]?.invoke(bytes) },
                        contentPadding = PaddingValues(horizontal = 10.dp, vertical = 6.dp),
                        colors = ButtonDefaults.textButtonColors(
                            contentColor = TextColor,
                            disabledContentColor = MutedColor.copy(alpha = 0.4f),
                        ),
                    ) {
                        Text(label, fontSize = 13.sp, fontWeight = FontWeight.Medium)
                    }
                }
            }
        }

        if (modeMenu) {
            InputModePopover(
                selectedMode = inputMode,
                effectiveMode = effectiveMode,
                slideOffsetPx = slideOffsetPx,
                onSelect = { mode ->
                    inputMode = mode
                    modeMenu = false
                },
                onDismiss = { modeMenu = false },
            )
        }
    }
}

private fun TerminalInputMode.summary(): String = when (this) {
    TerminalInputMode.Auto -> "Adapts to latency: Sync ≤80 ms, Predictive ≤220 ms, Line on slower paths."
    TerminalInputMode.Predictive -> "Shows typed characters immediately and reconciles remote echoes."
    TerminalInputMode.Sync -> "Waits for the host to echo each keystroke before displaying it."
    TerminalInputMode.Line -> "Compose a full command locally before sending it all at once."
}

@Composable
private fun InputModePopover(
    selectedMode: TerminalInputMode,
    effectiveMode: TerminalInputMode,
    slideOffsetPx: Int,
    onSelect: (TerminalInputMode) -> Unit,
    onDismiss: () -> Unit,
) {
    Box(
        Modifier
            .fillMaxSize()
            .zIndex(10f)
            .background(Color(0xFF040812).copy(alpha = 0.4f))
            .clickable(
                interactionSource = remember { MutableInteractionSource() },
                indication = null,
                onClick = onDismiss,
            ),
    ) {
        val cardShape = RoundedCornerShape(14.dp)
        Column(
            Modifier
                .align(Alignment.TopEnd)
                .statusBarsPadding()
                .padding(top = 44.dp, start = 16.dp, end = 10.dp)
                .offset { IntOffset(0, slideOffsetPx.coerceAtLeast(0)) }
                .widthIn(max = 320.dp)
                .clip(cardShape)
                .background(PanelColor, cardShape)
                .border(BorderStroke(1.dp, SelectedBorderColor.copy(alpha = 0.75f)), cardShape)
                .clickable(
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null,
                ) {}
                .padding(10.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(
                text = "Input mode",
                color = MutedColor,
                fontSize = 11.sp,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.padding(horizontal = 4.dp, vertical = 2.dp),
            )
            TerminalInputMode.entries.forEach { mode ->
                val isSelected = mode == selectedMode
                val itemShape = RoundedCornerShape(10.dp)
                Column(
                    Modifier
                        .fillMaxWidth()
                        .clip(itemShape)
                        .background(
                            if (isSelected) SelectedTabColor else ButtonSurfaceColor.copy(alpha = 0.65f),
                            itemShape,
                        )
                        .border(
                            BorderStroke(1.dp, if (isSelected) SelectedBorderColor else BorderColor),
                            itemShape,
                        )
                        .clickable { onSelect(mode) }
                        .padding(horizontal = 10.dp, vertical = 8.dp),
                ) {
                    Row(
                        Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            text = if (mode == TerminalInputMode.Auto) {
                                "Auto (currently ${effectiveMode.name})"
                            } else {
                                mode.name
                            },
                            color = TextColor,
                            fontSize = 13.sp,
                            fontWeight = FontWeight.SemiBold,
                            modifier = Modifier.weight(1f),
                        )
                        if (isSelected) {
                            Text(
                                text = "Active",
                                color = CyanColor,
                                fontSize = 11.sp,
                                fontWeight = FontWeight.Bold,
                            )
                        }
                    }
                    Spacer(Modifier.height(2.dp))
                    Text(
                        text = mode.summary(),
                        color = MutedColor,
                        fontSize = 11.sp,
                        lineHeight = 14.sp,
                    )
                }
            }
        }
    }
}

@Composable
private fun SheetStatusDot(color: Color) {
    Canvas(Modifier.size(14.dp)) {
        val center = Offset(size.width / 2f, size.height / 2f)
        val unit = size.minDimension / 16f
        drawCircle(color = color.copy(alpha = 0.27f), radius = 7.5f * unit, center = center)
        drawCircle(color = color, radius = 4.2f * unit, center = center)
        drawCircle(color = Color(0xFFF4FCFF), radius = 1.8f * unit, center = center)
    }
}

@Composable
private fun HeaderIconButton(
    description: String,
    onClick: () -> Unit,
    content: @Composable () -> Unit,
) {
    Box(
        modifier = Modifier
            .size(28.dp)
            .clip(CircleShape)
            .background(ButtonSurfaceColor, CircleShape)
            .border(BorderStroke(1.dp, BorderColor), CircleShape)
            .semantics {
                contentDescription = description
                role = Role.Button
            }
            .clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        content()
    }
}

private fun Modifier.padding(
    fromTop: androidx.compose.ui.unit.Dp,
    start: androidx.compose.ui.unit.Dp,
    end: androidx.compose.ui.unit.Dp,
    bottom: androidx.compose.ui.unit.Dp,
): Modifier = this.padding(start = start, top = fromTop, end = end, bottom = bottom)

@Composable
private fun LineComposer(value: String, focusNonce: Int, onChange: (String) -> Unit, onSend: () -> Unit) {
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    LaunchedEffect(focusNonce) {
        if (focusNonce > 0) {
            focus.requestFocus()
            keyboard?.show()
        }
    }
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 10.dp, vertical = 4.dp)
            .clip(RoundedCornerShape(12.dp))
            .background( Color(0xFF0D1526) )
            .border(BorderStroke(1.dp, BorderColor), RoundedCornerShape(12.dp))
            .padding(horizontal = 12.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BasicTextField(
            value = value,
            onValueChange = onChange,
            singleLine = true,
            textStyle = TextStyle(color = TextColor, fontFamily = FontFamily.Monospace, fontSize = 15.sp),
            cursorBrush = SolidColor(CyanColor),
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
            keyboardActions = KeyboardActions(onSend = { onSend() }),
            modifier = Modifier.weight(1f).focusRequester(focus),
            decorationBox = { inner ->
                Box {
                    if (value.isEmpty()) Text("Enter command…", color = MutedColor)
                    inner()
                }
            },
        )
        TextButton(onClick = onSend, colors = ButtonDefaults.textButtonColors(contentColor = CyanColor)) {
            Text("Send", fontWeight = FontWeight.Bold)
        }
    }
}
