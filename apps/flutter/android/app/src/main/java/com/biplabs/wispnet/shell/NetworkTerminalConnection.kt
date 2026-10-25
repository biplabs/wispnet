// SPDX-License-Identifier: Apache-2.0
package com.biplabs.wispnet.shell

import android.os.Handler
import android.os.Looper
import android.util.Base64
import com.biplabs.wispnet.ShellRpc
import org.json.JSONObject
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit

/** Bytes and terminal events over Wispnet's private Binder client, never a new identity. */
internal class NetworkTerminalConnection(
    private val rpc: ShellRpc,
    private val target: ShellTarget,
    private val sessionName: String,
    private val onBytes: (ByteArray, Boolean) -> Unit,
    private val onState: (ConnectionState) -> Unit,
    private val onTransportPathChanged: (TransportPathStatus) -> Unit,
    private val onConnectionError: (String) -> Unit,
    private val onTerminalState: (Boolean, Boolean) -> Unit,
) : WispTerminalConnection {
    private val work = Executors.newSingleThreadScheduledExecutor()
    private val main = Handler(Looper.getMainLooper())
    @Volatile private var closed = false
    private var handle: String? = null
    private var poll: ScheduledFuture<*>? = null
    private var columns = 80
    private var rows = 24

    private fun deliver(action: () -> Unit) { main.post { if (!closed) action() } }
    private fun execute(action: () -> Unit) {
        if (closed) return
        try { work.execute { if (!closed) action() } } catch (_: RejectedExecutionException) {}
    }

    override fun connect() = execute {
        try {
            deliver { onState(ConnectionState.Connecting) }
            val reply = rpc.request("shell_open", target.json().put("session", sessionName)
                .put("cols", columns).put("rows", rows))
            val opened = reply.getString("session")
            if (closed) {
                rpc.request("shell_close", JSONObject().put("session", opened))
                return@execute
            }
            handle = opened
            deliver { onState(ConnectionState.Attached) }
            consume(reply)
            poll = work.scheduleWithFixedDelay({
                val active = handle
                if (closed || active == null) return@scheduleWithFixedDelay
                try { consume(rpc.request("shell_poll", JSONObject().put("session", active))) }
                catch (error: Exception) { disconnected(error) }
            }, 0, 20, TimeUnit.MILLISECONDS)
        } catch (error: Exception) { disconnected(error) }
    }

    private fun consume(reply: JSONObject) {
        val latency = reply.optInt("latency_ms", -1).takeIf { it >= 0 }
        val path = reply.optString("path", "unknown")
        deliver { onTransportPathChanged(TransportPathStatus(path, latency)) }
        val latestEcho = reply.optBoolean("echo_enabled", false)
        val latestCanonical = reply.optBoolean("canonical", false)
        if (reply.has("echo_enabled")) {
            deliver { onTerminalState(latestEcho, latestCanonical) }
        }
        val events = reply.optJSONArray("events")
        if (events != null) for (index in 0 until events.length()) {
            val event = events.getJSONObject(index)
            when (event.optString("type")) {
                "output", "scrollback" -> {
                    val bytes = Base64.decode(event.getString("data_b64"), Base64.DEFAULT)
                    val replay = event.getString("type") == "scrollback"
                    deliver { onBytes(bytes, replay) }
                }
                "terminal_state" -> {
                    // Queued history can contain an old echo-on event after
                    // the host has already disabled echo for a password. Never
                    // let that historical event override the live safe state.
                    val echo = event.optBoolean("echo", false) && latestEcho
                    val canonical = event.optBoolean("canonical", false) && latestCanonical
                    deliver { onTerminalState(echo, canonical) }
                }
                "exit" -> {
                    val notice = "\r\n[Shell exited with status ${event.optInt("code", 0)}]\r\n".toByteArray()
                    deliver { onBytes(notice, false); onState(ConnectionState.Exited) }
                    release()
                }
                "error" -> {
                    val message = event.optString("message", "Shell connection failed")
                    deliver { onConnectionError(message) }
                }
            }
        }
        when (reply.optString("state")) {
            "disconnected" -> disconnected(IllegalStateException("Shell disconnected. Reconnect to resume."))
            // The exit event follows any queued output. Keep polling until it
            // arrives even if the process's metadata already says exited.
            "exited" -> Unit
        }
    }

    private fun release() {
        poll?.cancel(false)
        poll = null
        val old = handle
        handle = null
        if (old != null) try { rpc.request("shell_close", JSONObject().put("session", old)) } catch (_: Exception) {}
    }

    private fun disconnected(error: Exception) {
        release()
        deliver {
            onTerminalState(false, false)
            onConnectionError(error.message ?: "Shell connection failed")
            onState(ConnectionState.Disconnected)
        }
    }

    override fun sendInput(data: String) = sendBytes(data.toByteArray(Charsets.UTF_8))
    override fun sendBytes(bytes: ByteArray) {
        // Binder payloads remain bounded, including large clipboard pastes.
        var offset = 0
        while (offset < bytes.size) {
            val end = minOf(offset + 8 * 1024, bytes.size)
            val chunk = bytes.copyOfRange(offset, end)
            execute {
                val active = handle ?: return@execute
                try { rpc.request("shell_input", JSONObject().put("session", active)
                    .put("data_b64", Base64.encodeToString(chunk, Base64.NO_WRAP))) }
                catch (error: Exception) { disconnected(error) }
            }
            offset = end
        }
    }

    override fun resize(cols: Int, rows: Int) = execute {
        columns = cols.coerceIn(2, 500)
        this.rows = rows.coerceIn(2, 300)
        val active = handle ?: return@execute
        try { rpc.request("shell_resize", JSONObject().put("session", active)
            .put("cols", columns).put("rows", this.rows)) }
        catch (error: Exception) { disconnected(error) }
    }

    override fun close() {
        if (closed) return
        closed = true
        main.removeCallbacksAndMessages(null)
        poll?.cancel(false)
        try { work.execute { release() } } catch (_: RejectedExecutionException) {}
        work.shutdown()
    }
}
