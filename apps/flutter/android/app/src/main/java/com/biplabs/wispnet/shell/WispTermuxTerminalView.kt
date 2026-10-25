// SPDX-License-Identifier: AGPL-3.0-only
// Adapted from Biplabs WispShell (7028342); see assets/licenses/WispShell-NOTICE.txt.
package com.biplabs.wispnet.shell

import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.os.SystemClock
import android.text.InputType
import android.util.Log
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputConnectionWrapper
import android.view.inputmethod.InputMethodManager
import android.view.ViewConfiguration
import android.widget.EditText
import android.widget.FrameLayout
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import com.biplabs.wispnet.ShellRpc
import com.termux.terminal.TerminalEmulator
import com.termux.terminal.TerminalOutput
import com.termux.terminal.TerminalSession
import com.termux.terminal.TerminalSessionClient
import com.termux.view.TerminalView
import com.termux.view.TerminalViewClient

private const val TAG = "WispTermuxTerminal"
private const val RESIZE_SETTLE_DELAY_MS = 140L
private const val REMOTE_RESIZE_REDRAW_DEFER_MS = 220L
private const val REMOTE_RESIZE_REDRAW_QUIET_MS = 80L
private const val MAX_PREDICTIVE_ECHO_BYTES = 256
private const val LOCAL_INPUT_FLUSH_DELAY_MS = 260L

data class TransportPathStatus(
    val path: String,
    val latencyMs: Int?,
)

@Composable
fun WispTermuxTerminalView(
    modifier: Modifier = Modifier,
    sessionName: String = "main",
    active: Boolean = true,
    reconnectNonce: Int = 0,
    target: ShellTarget,
    rpc: ShellRpc,
    inputMode: TerminalInputMode = TerminalInputMode.Predictive,
    onConnectionState: (ConnectionState) -> Unit = {},
    onConnectionError: (String) -> Unit = {},
    onTransportPathChanged: (TransportPathStatus?) -> Unit = {},
    onTitleChanged: (String) -> Unit = {},
    onTerminalModeChanged: (Boolean, Boolean) -> Unit = { _, _ -> },
    onSendInputReady: (((String) -> Unit)?) -> Unit = {},
    onKeyboardRequest: () -> Unit = {},
) {
    val lifecycleOwner = LocalLifecycleOwner.current
    val holder = remember(sessionName, reconnectNonce, target, rpc) {
        TermuxTerminalHolder(
            target,
            rpc,
            sessionName,
            inputMode,
            onConnectionState,
            onConnectionError,
            onTransportPathChanged,
            onTitleChanged,
            onTerminalModeChanged,
            onKeyboardRequest,
        )
    }
    LaunchedEffect(holder, inputMode) {
        holder.setInputMode(inputMode)
    }
    DisposableEffect(lifecycleOwner, holder) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                holder.reconnectIfDisconnected()
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
        }
    }
    DisposableEffect(holder) {
        onSendInputReady { input -> holder.sendInput(input) }
        onDispose {
            onSendInputReady(null)
            holder.close()
        }
    }

    AndroidView(
        modifier = modifier,
        factory = { context ->
            val terminalView = TerminalView(context, null).apply {
                setBackgroundColor(Color.BLACK)
                setTextSize(28)
                setTypeface(Typeface.MONOSPACE)
                isFocusable = true
                isFocusableInTouchMode = true
            }
            val inputView = TerminalInputEditText(context) { input ->
                holder.sendInputFromIme(input)
            }
            FrameLayout(context).apply {
                addView(
                    terminalView,
                    FrameLayout.LayoutParams(
                        FrameLayout.LayoutParams.MATCH_PARENT,
                        FrameLayout.LayoutParams.MATCH_PARENT,
                    ),
                )
                addView(inputView, FrameLayout.LayoutParams(1, 1))
                holder.attachView(terminalView, inputView)
            }
        },
        update = { container ->
            val terminalView = container.getChildAt(0) as TerminalView
            terminalView.alpha = if (active) 1f else 0f
            terminalView.isEnabled = active
            holder.setActive(active)
        },
    )
}

private class TermuxTerminalHolder(
    private val target: ShellTarget,
    private val rpc: ShellRpc,
    private val sessionName: String,
    private var inputMode: TerminalInputMode,
    private val onConnectionState: (ConnectionState) -> Unit,
    private val onConnectionError: (String) -> Unit,
    private val onTransportPathChanged: (TransportPathStatus?) -> Unit,
    private val onTitleChanged: (String) -> Unit,
    private val onTerminalModeChanged: (Boolean, Boolean) -> Unit,
    private val onKeyboardRequest: () -> Unit,
) : TerminalSessionClient, TerminalViewClient {
    private var view: TerminalView? = null
    private var inputView: TerminalInputEditText? = null
    private var emulator: TerminalEmulator? = null
    private var dummySession: TerminalSession? = null
    private var connection: WispTerminalConnection? = null
    private var layoutListener: View.OnLayoutChangeListener? = null
    private var pendingResize: Runnable? = null
    private var pendingScreenRefresh: Runnable? = null
    private var deferScreenUpdatesUntil = 0L
    private var lastSentColumns: Int? = null
    private var lastSentRows: Int? = null
    private var lastTapUpTime = 0L
    private var keyboardVisible = false
    private var keyboardAutoShown = false
    private var pendingInputFlush: Runnable? = null
    private val pendingRemoteInput = StringBuilder()
    private val pendingLocalEcho = mutableListOf<Byte>()
    private var pendingLocalErases = 0
    @Volatile private var suppressRemoteWrites = false
    @Volatile private var connectionState = ConnectionState.Connecting
    private var echoEnabled = false
    private var canonicalInput = false
    private var replayStarted = false

    fun attachView(newView: TerminalView, newInputView: TerminalInputEditText) {
        view?.let { oldView ->
            layoutListener?.let(oldView::removeOnLayoutChangeListener)
            pendingResize?.let(oldView::removeCallbacks)
            pendingScreenRefresh?.let(oldView::removeCallbacks)
        }
        pendingResize = null
        pendingScreenRefresh = null
        lastSentColumns = null
        lastSentRows = null
        keyboardAutoShown = false
        view = newView
        inputView = newInputView
        newView.setTerminalViewClient(this)
        newView.setOnTouchListener { _, event ->
            if (event.actionMasked == MotionEvent.ACTION_UP) {
                val now = event.eventTime
                if (now - lastTapUpTime <= ViewConfiguration.getDoubleTapTimeout()) {
                    lastTapUpTime = 0L
                    requestKeyboard()
                } else {
                    lastTapUpTime = now
                }
            }
            false
        }

        val remoteOutput = RemoteTerminalOutput(
            onWrite = { bytes ->
                if (suppressRemoteWrites) return@RemoteTerminalOutput
                connection?.sendBytes(bytes)
            },
            onTitle = { title -> updateTitle(title) },
        )
        val remoteEmulator = TerminalEmulator(
            remoteOutput,
            80,
            24,
            2000,
            this,
        )
        emulator = remoteEmulator
        dummySession = TerminalSession(
            "/system/bin/sh",
            "/",
            arrayOf("sh", "-c", "while true; do sleep 3600; done"),
            emptyArray(),
            2000,
            this,
        )
        newView.attachSession(dummySession)
        newView.mEmulator = remoteEmulator
        newView.requestFocus()

        startConnection(newView, remoteEmulator)
        layoutListener = View.OnLayoutChangeListener { changedView, left, top, right, bottom, oldLeft, oldTop, oldRight, oldBottom ->
            if (right - left != oldRight - oldLeft || bottom - top != oldBottom - oldTop) {
                (changedView as? TerminalView)?.let(::scheduleSizeSync)
            }
        }.also(newView::addOnLayoutChangeListener)
        newView.post { syncSizeFromView(newView) }
    }

    @Synchronized
    fun reconnectIfDisconnected() {
        if (connectionState != ConnectionState.Disconnected) return
        val terminalView = view ?: return
        val remoteEmulator = emulator ?: return
        startConnection(terminalView, remoteEmulator)
    }

    @Synchronized
    private fun startConnection(
        terminalView: TerminalView,
        remoteEmulator: TerminalEmulator,
    ) {
        connection?.close()
        connectionState = ConnectionState.Connecting
        echoEnabled = false
        canonicalInput = false
        replayStarted = false
        val nextConnection = createConnection(terminalView, remoteEmulator)
        connection = nextConnection
        val sizeSource = dummySession?.emulator
        if (sizeSource != null) {
            nextConnection.resize(sizeSource.mColumns, sizeSource.mRows)
        }
        nextConnection.connect()
    }

    private fun createConnection(
        newView: TerminalView,
        remoteEmulator: TerminalEmulator,
    ): WispTerminalConnection {
        val onBytes: (ByteArray, Boolean) -> Unit = { bytes, fromScrollback ->
            inferTitle(bytes)?.let { updateTitle(it) }
            newView.post {
                val displayBytes = if (fromScrollback) {
                    if (!replayStarted) {
                        remoteEmulator.reset()
                        replayStarted = true
                    }
                    pendingRemoteInput.clear()
                    pendingLocalEcho.clear()
                    pendingLocalErases = 0
                    bytes
                } else {
                    flushPendingInput()
                    reconcileLocalEcho(bytes)
                }
                if (displayBytes.isEmpty()) {
                    newView.onScreenUpdated()
                    return@post
                }
                if (fromScrollback) {
                    suppressRemoteWrites = true
                }
                try {
                    remoteEmulator.append(displayBytes, displayBytes.size)
                    if (shouldDeferScreenUpdate()) {
                        scheduleDeferredScreenRefresh(newView)
                    } else {
                        newView.onScreenUpdated()
                    }
                } finally {
                    if (fromScrollback) {
                        suppressRemoteWrites = false
                    }
                }
            }
        }
        val onState: (ConnectionState) -> Unit = { state ->
            connectionState = state
            if (state == ConnectionState.Attached) {
                resizeConnectionToView()
            }
            onConnectionState(state)
        }
        val onError: (String) -> Unit = { message ->
            onConnectionError(message)
        }
        val onTransportPath: (TransportPathStatus) -> Unit = { status ->
            onTransportPathChanged(status)
        }
        return NetworkTerminalConnection(
            rpc = rpc,
            target = target,
            sessionName = sessionName,
            onBytes = onBytes,
            onState = onState,
            onTransportPathChanged = onTransportPath,
            onConnectionError = onError,
            onTerminalState = { echo, canonical ->
                echoEnabled = echo
                canonicalInput = canonical
                onTerminalModeChanged(echo, canonical)
                if (!echo || !canonical) {
                    flushPendingInput()
                    pendingLocalEcho.clear()
                    pendingLocalErases = 0
                }
            },
        )
    }

    fun close() {
        view?.let { terminalView ->
            layoutListener?.let(terminalView::removeOnLayoutChangeListener)
            pendingResize?.let(terminalView::removeCallbacks)
            pendingScreenRefresh?.let(terminalView::removeCallbacks)
            pendingInputFlush?.let(terminalView::removeCallbacks)
        }
        layoutListener = null
        pendingResize = null
        pendingScreenRefresh = null
        pendingInputFlush = null
        deferScreenUpdatesUntil = 0L
        lastSentColumns = null
        lastSentRows = null
        keyboardAutoShown = false
        pendingRemoteInput.clear()
        pendingLocalEcho.clear()
        pendingLocalErases = 0
        connection?.close()
        onTransportPathChanged(null)
        dummySession?.finishIfRunning()
        connection = null
        dummySession = null
        emulator = null
        view = null
        inputView = null
    }

    fun setActive(active: Boolean) {
        val terminalView = view ?: return
        terminalView.isEnabled = active
        if (active && !keyboardAutoShown && inputMode != TerminalInputMode.Line) {
            keyboardAutoShown = true
            showKeyboard()
        }
    }

    fun setInputMode(mode: TerminalInputMode) {
        if (inputMode == mode) return
        inputMode = mode
        if (mode == TerminalInputMode.Line) {
            hideKeyboard()
        } else if (view?.isEnabled == true) {
            showKeyboard()
        }
    }

    private fun sendRemote(text: String, predictiveEcho: Boolean = false): Boolean {
        if (predictiveEcho && shouldPredictLocalEcho(text)) {
            appendLocalEcho(text, queueRemoteInput = true)
            scheduleInputFlush()
            return true
        }
        flushPendingInput()
        connection?.sendInput(text)
        return true
    }

    fun sendInput(text: String): Boolean {
        return sendRemote(text)
    }

    fun sendInputFromIme(text: String): Boolean {
        return when (inputMode) {
            TerminalInputMode.Auto -> sendRemote(text, predictiveEcho = true)
            TerminalInputMode.Sync -> sendRemote(text)
            TerminalInputMode.Line -> true
            TerminalInputMode.Predictive -> sendRemote(text, predictiveEcho = true)
        }
    }

    private fun showKeyboard() {
        val terminalView = view ?: return
        val keyboardTarget = inputView ?: terminalView
        terminalView.post {
            keyboardTarget.requestFocus()
            val imm =
                terminalView.context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
            imm.restartInput(keyboardTarget)
            imm.showSoftInput(keyboardTarget, InputMethodManager.SHOW_IMPLICIT)
            keyboardVisible = true
        }
    }

    private fun hideKeyboard() {
        val terminalView = view ?: return
        val keyboardTarget = inputView ?: terminalView
        terminalView.post {
            val imm =
                terminalView.context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
            imm.hideSoftInputFromWindow(keyboardTarget.windowToken, 0)
            terminalView.requestFocus()
            keyboardVisible = false
        }
    }

    private fun toggleKeyboard() {
        if (keyboardVisible) {
            hideKeyboard()
        } else {
            showKeyboard()
        }
    }

    private fun requestKeyboard() {
        if (inputMode == TerminalInputMode.Line) {
            onKeyboardRequest()
        } else {
            toggleKeyboard()
        }
    }

    override fun onTextChanged(changedSession: TerminalSession?) {
        view?.onScreenUpdated()
    }

    override fun onTitleChanged(changedSession: TerminalSession?) = Unit
    override fun onSessionFinished(finishedSession: TerminalSession?) = Unit
    override fun onCopyTextToClipboard(session: TerminalSession?, text: String?) = Unit
    override fun onPasteTextFromClipboard(session: TerminalSession?) = Unit
    override fun onBell(session: TerminalSession?) = Unit
    override fun onColorsChanged(session: TerminalSession?) = Unit
    override fun onTerminalCursorStateChange(state: Boolean) = Unit
    override fun getTerminalCursorStyle(): Int? = null

    override fun onScale(scale: Float): Float = scale
    override fun onSingleTapUp(event: MotionEvent?) = Unit

    override fun shouldBackButtonBeMappedToEscape(): Boolean = false
    override fun shouldEnforceCharBasedInput(): Boolean = false
    override fun shouldUseCtrlSpaceWorkaround(): Boolean = false
    override fun isTerminalViewSelected(): Boolean = true
    override fun copyModeChanged(copyMode: Boolean) = Unit
    override fun onKeyDown(keyCode: Int, event: KeyEvent?, session: TerminalSession?): Boolean {
        return when (keyCode) {
            KeyEvent.KEYCODE_ENTER -> sendRemote("\r")
            KeyEvent.KEYCODE_DEL -> sendRemote("\u007f")
            KeyEvent.KEYCODE_TAB -> sendRemote("\t")
            KeyEvent.KEYCODE_ESCAPE -> sendRemote("\u001b")
            KeyEvent.KEYCODE_DPAD_UP -> sendRemote("\u001b[A")
            KeyEvent.KEYCODE_DPAD_DOWN -> sendRemote("\u001b[B")
            KeyEvent.KEYCODE_DPAD_RIGHT -> sendRemote("\u001b[C")
            KeyEvent.KEYCODE_DPAD_LEFT -> sendRemote("\u001b[D")
            else -> sendKeyEventCodePoint(event)
        }
    }

    private fun sendKeyEventCodePoint(event: KeyEvent?): Boolean {
        val codePoint = event?.getUnicodeChar(event.metaState) ?: return false
        if (codePoint <= 0 || codePoint and KeyCharacterMap.COMBINING_ACCENT != 0) {
            return false
        }
        return sendRemote(String(Character.toChars(codePoint)))
    }

    private fun shouldPredictLocalEcho(text: String): Boolean {
        if (connectionState != ConnectionState.Attached) return false
        // Host termios is authoritative. Password prompts/raw-mode apps never
        // receive local predictive echo, even when Predictive is selected.
        if (!echoEnabled || !canonicalInput) return false
        if (emulator?.isAlternateBufferActive == true) return false
        if (text.isEmpty()) return false
        if (pendingRemoteInput.length >= MAX_PREDICTIVE_ECHO_BYTES) return false
        return text.all { it in ' '..'~' || it == '\u007f' }
    }

    private fun appendLocalEcho(text: String, queueRemoteInput: Boolean = false) {
        val remoteEmulator = emulator ?: return
        val terminalView = view ?: return
        if (text == "\u007f") {
            appendLocalBackspace(remoteEmulator, terminalView, queueRemoteInput)
            return
        }
        val bytes = text.toByteArray(Charsets.UTF_8)
        if (pendingRemoteInput.length + bytes.size > MAX_PREDICTIVE_ECHO_BYTES) {
            pendingRemoteInput.clear()
            return
        }
        if (queueRemoteInput) {
            pendingRemoteInput.append(text)
        }
        remoteEmulator.append(bytes, bytes.size)
        terminalView.onScreenUpdated()
    }

    private fun appendLocalBackspace(
        remoteEmulator: TerminalEmulator,
        terminalView: TerminalView,
        queueRemoteInput: Boolean,
    ) {
        if (pendingRemoteInput.isNotEmpty() && pendingRemoteInput.last() != '\u007f') {
            pendingRemoteInput.deleteAt(pendingRemoteInput.length - 1)
        } else if (queueRemoteInput) {
            pendingRemoteInput.append("\u007f")
        }
        val bytes = "\b \b".toByteArray(Charsets.UTF_8)
        remoteEmulator.append(bytes, bytes.size)
        terminalView.onScreenUpdated()
    }

    private fun scheduleInputFlush() {
        val terminalView = view ?: return
        pendingInputFlush?.let(terminalView::removeCallbacks)
        pendingInputFlush = Runnable {
            pendingInputFlush = null
            flushPendingInput()
        }.also { terminalView.postDelayed(it, LOCAL_INPUT_FLUSH_DELAY_MS) }
    }

    private fun flushPendingInput() {
        val terminalView = view
        if (terminalView != null) {
            pendingInputFlush?.let(terminalView::removeCallbacks)
        }
        pendingInputFlush = null
        if (pendingRemoteInput.isEmpty()) return
        val text = pendingRemoteInput.toString()
        pendingRemoteInput.clear()
        expectRemoteEcho(text)
        connection?.sendInput(text)
    }

    private fun expectRemoteEcho(text: String) {
        text.forEach { char ->
            if (char == '\u007f') {
                pendingLocalErases += 1
            } else {
                pendingLocalEcho.addAll(char.toString().toByteArray(Charsets.UTF_8).toList())
            }
        }
        if (pendingLocalEcho.size > MAX_PREDICTIVE_ECHO_BYTES) {
            pendingLocalEcho.clear()
        }
    }

    private fun reconcileLocalEcho(bytes: ByteArray): ByteArray {
        var consumed = consumeRemoteErases(bytes)
        while (
            consumed < bytes.size &&
            pendingLocalEcho.isNotEmpty() &&
            pendingLocalEcho.first() == bytes[consumed]
        ) {
            pendingLocalEcho.removeAt(0)
            consumed += 1
        }

        if (consumed == bytes.size) return ByteArray(0)
        val remaining = bytes.copyOfRange(consumed, bytes.size)
        if (pendingLocalEcho.isEmpty()) return remaining

        pendingLocalEcho.clear()
        return remaining
    }

    private fun consumeRemoteErases(bytes: ByteArray): Int {
        var consumed = 0
        while (
            pendingLocalErases > 0 &&
            consumed + 2 < bytes.size &&
            bytes[consumed] == '\b'.code.toByte() &&
            bytes[consumed + 1] == ' '.code.toByte() &&
            bytes[consumed + 2] == '\b'.code.toByte()
        ) {
            consumed += 3
            pendingLocalErases -= 1
        }
        return consumed
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent?): Boolean = false
    override fun onLongPress(event: MotionEvent?): Boolean = false
    override fun readControlKey(): Boolean = false
    override fun readAltKey(): Boolean = false
    override fun readShiftKey(): Boolean = false
    override fun readFnKey(): Boolean = false
    override fun onCodePoint(
        codePoint: Int,
        ctrlDown: Boolean,
        session: TerminalSession?,
    ): Boolean {
        val remoteCodePoint = if (ctrlDown) {
            when (codePoint) {
                in 'a'.code..'z'.code -> codePoint - 'a'.code + 1
                in 'A'.code..'Z'.code -> codePoint - 'A'.code + 1
                ' '.code, '2'.code -> 0
                '['.code, '3'.code -> 27
                '\\'.code, '4'.code -> 28
                ']'.code, '5'.code -> 29
                '^'.code, '6'.code -> 30
                '_'.code, '7'.code, '/'.code -> 31
                '8'.code -> 127
                else -> codePoint
            }
        } else {
            codePoint
        }
        return sendRemote(String(Character.toChars(remoteCodePoint)))
    }

    override fun onEmulatorSet() {
        val terminalView = view ?: return
        val remoteEmulator = emulator ?: return
        val sizeSource = dummySession?.emulator ?: return
        remoteEmulator.resize(sizeSource.mColumns, sizeSource.mRows)
        terminalView.mEmulator = remoteEmulator
    }

    private fun syncSizeFromView(terminalView: TerminalView) {
        if (terminalView.width == 0 || terminalView.height == 0) return
        val oldColumns = dummySession?.emulator?.mColumns
        terminalView.updateSize()
        emulator?.let { terminalView.mEmulator = it }
        val newColumns = dummySession?.emulator?.mColumns ?: return
        if (oldColumns == null || oldColumns != newColumns) {
            resizeConnectionToView()
        }
    }

    private fun scheduleSizeSync(terminalView: TerminalView) {
        pendingResize?.let(terminalView::removeCallbacks)
        pendingResize = Runnable {
            pendingResize = null
            syncSizeFromView(terminalView)
        }.also { terminalView.postDelayed(it, RESIZE_SETTLE_DELAY_MS) }
    }

    private fun resizeConnectionToView() {
        val sizeSource = dummySession?.emulator ?: return
        if (lastSentColumns == sizeSource.mColumns && lastSentRows == sizeSource.mRows) return
        lastSentColumns = sizeSource.mColumns
        lastSentRows = sizeSource.mRows
        deferScreenUpdatesUntil = SystemClock.uptimeMillis() + REMOTE_RESIZE_REDRAW_DEFER_MS
        connection?.resize(sizeSource.mColumns, sizeSource.mRows)
    }

    private fun shouldDeferScreenUpdate(): Boolean {
        val now = SystemClock.uptimeMillis()
        if (now >= deferScreenUpdatesUntil) return false
        deferScreenUpdatesUntil = now + REMOTE_RESIZE_REDRAW_QUIET_MS
        return true
    }

    private fun scheduleDeferredScreenRefresh(terminalView: TerminalView) {
        pendingScreenRefresh?.let(terminalView::removeCallbacks)
        val delay = (deferScreenUpdatesUntil - SystemClock.uptimeMillis()).coerceAtLeast(0L)
        pendingScreenRefresh = Runnable {
            pendingScreenRefresh = null
            deferScreenUpdatesUntil = 0L
            terminalView.onScreenUpdated()
        }.also { terminalView.postDelayed(it, delay) }
    }

    private fun updateTitle(title: String?) {
        val normalized = title?.trim()?.takeIf { it.isNotEmpty() } ?: return
        onTitleChanged(folderName(normalized))
    }

    override fun logError(tag: String?, message: String?) {
        Log.e(tag ?: TAG, message.orEmpty())
    }

    override fun logWarn(tag: String?, message: String?) {
        Log.w(tag ?: TAG, message.orEmpty())
    }

    override fun logInfo(tag: String?, message: String?) {
        Log.i(tag ?: TAG, message.orEmpty())
    }

    override fun logDebug(tag: String?, message: String?) {
        Log.d(tag ?: TAG, message.orEmpty())
    }

    override fun logVerbose(tag: String?, message: String?) {
        Log.v(tag ?: TAG, message.orEmpty())
    }
    override fun logStackTraceWithMessage(tag: String?, message: String?, e: Exception?) {
        Log.e(tag ?: TAG, message.orEmpty(), e)
    }

    override fun logStackTrace(tag: String?, e: Exception?) {
        Log.e(tag ?: TAG, "terminal exception", e)
    }
}

private class TerminalInputEditText(
    context: Context,
    private val onInput: (String) -> Boolean,
) : EditText(context) {
    init {
        alpha = 0.01f
        setBackgroundColor(Color.TRANSPARENT)
        setTextColor(Color.TRANSPARENT)
        isCursorVisible = false
        isFocusable = true
        isFocusableInTouchMode = true
        setSingleLine(true)
    }

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        val connection = super.onCreateInputConnection(outAttrs)
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or
            InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        outAttrs.imeOptions = EditorInfo.IME_ACTION_NONE or
            EditorInfo.IME_FLAG_NO_EXTRACT_UI or EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
        outAttrs.privateImeOptions = "restrictDirectWritingArea=true"
        return TerminalInputConnection(connection, onInput)
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent?): Boolean {
        return when (keyCode) {
            KeyEvent.KEYCODE_ENTER -> onInput("\r")
            KeyEvent.KEYCODE_DEL -> onInput("\u007f")
            KeyEvent.KEYCODE_TAB -> onInput("\t")
            KeyEvent.KEYCODE_ESCAPE -> onInput("\u001b")
            else -> super.onKeyDown(keyCode, event)
        }
    }
}

private class TerminalInputConnection(
    target: InputConnection,
    private val onInput: (String) -> Boolean,
) : InputConnectionWrapper(target, true) {
    private val composition = TerminalComposition()

    private fun syncComposingText(newValue: String) {
        composition.update(newValue).forEach { onInput(it) }
    }

    override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
        val value = text?.toString().orEmpty()
        syncComposingText(value)
        composition.finish()
        return true
    }

    override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
        val value = text?.toString().orEmpty()
        syncComposingText(value)
        return true
    }

    override fun finishComposingText(): Boolean {
        composition.finish()
        return true
    }

    override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
        if (beforeLength > 0) {
            repeat(beforeLength) {
                composition.deleted()
                onInput("\u007f")
            }
            return true
        }
        return super.deleteSurroundingText(beforeLength, afterLength)
    }

    override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int): Boolean {
        return deleteSurroundingText(beforeLength, afterLength)
    }

    override fun sendKeyEvent(event: KeyEvent?): Boolean {
        if (event?.action != KeyEvent.ACTION_DOWN) {
            return true
        }
        return when (event.keyCode) {
            KeyEvent.KEYCODE_ENTER -> onInput("\r")
            KeyEvent.KEYCODE_DEL -> {
                composition.deleted()
                onInput("\u007f")
            }
            KeyEvent.KEYCODE_TAB -> onInput("\t")
            KeyEvent.KEYCODE_ESCAPE -> onInput("\u001b")
            else -> {
                val codePoint = event.getUnicodeChar(event.metaState)
                codePoint > 0 && onInput(String(Character.toChars(codePoint)))
            }
        }
    }

    override fun performEditorAction(editorAction: Int): Boolean {
        return onInput("\r")
    }
}

private class RemoteTerminalOutput(
    private val onWrite: (ByteArray) -> Unit,
    private val onTitle: (String?) -> Unit,
) : TerminalOutput() {
    override fun write(data: ByteArray, offset: Int, count: Int) {
        onWrite(data.copyOfRange(offset, offset + count))
    }

    override fun titleChanged(oldTitle: String?, newTitle: String?) {
        onTitle(newTitle)
    }
    override fun onCopyTextToClipboard(text: String?) = Unit
    override fun onPasteTextFromClipboard() = Unit
    override fun onBell() = Unit
    override fun onColorsChanged() = Unit
}

private fun inferTitle(bytes: ByteArray): String? {
    val text = bytes.toString(Charsets.UTF_8)
        .replace(Regex("\\u001b\\[[0-?]*[ -/]*[@-~]"), "")
        .replace(Regex("\\u001b\\][^\\u0007]*(\\u0007|\\u001b\\\\)"), "")
    val directory = Regex("""directory:\s+([^\r\n ]+)""")
        .find(text)
        ?.groupValues
        ?.getOrNull(1)
    if (!directory.isNullOrBlank()) {
        return folderName(directory)
    }
    val promptPath = Regex("""(?:^|[\s.])(~?/[A-Za-z0-9._~/-]+)""")
        .findAll(text)
        .map { it.groupValues[1] }
        .lastOrNull { it.contains('/') }
    return promptPath?.let(::folderName)
}

private fun folderName(pathOrTitle: String): String {
    val cleaned = pathOrTitle
        .trim()
        .trimEnd('/', ':')
    return cleaned.substringAfterLast('/').ifBlank { cleaned }
}
