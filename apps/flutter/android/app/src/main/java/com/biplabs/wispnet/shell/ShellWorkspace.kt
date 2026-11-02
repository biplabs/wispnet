// SPDX-License-Identifier: Apache-2.0
package com.biplabs.wispnet.shell

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.os.Handler
import android.os.Looper
import com.biplabs.wispnet.ShellRpc
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.ConcurrentHashMap

internal data class ShellTab(val target: ShellTarget, val session: String) {
    val id: String get() = "${target.host}/${target.service}/${target.incarnation}/$session"
    fun json(): JSONObject = JSONObject().put("target", target.json()).put("session", session)
}

/** Keeps terminal tabs and PTY streams alive while the sheet is minimized. */
object ShellWorkspace {
    private var rpc: ShellRpc? = null
    private val holders = ConcurrentHashMap<String, TermuxTerminalHolder>()
    private var networkManager: ConnectivityManager? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private val networkHandler by lazy { Handler(Looper.getMainLooper()) }
    internal var tabs: List<ShellTab> = emptyList()
    internal var activeHost = ""
    internal var selected = ""
    internal var minimized = false
    internal val selectedByHost = ConcurrentHashMap<String, String>()
    internal val minimizedHosts = ConcurrentHashMap<String, Boolean>()
    internal val titles = ConcurrentHashMap<String, String>()
    internal val paths = ConcurrentHashMap<String, TransportPathStatus>()
    internal val states = ConcurrentHashMap<String, ConnectionState>()
    internal val inputStates = ConcurrentHashMap<String, Pair<Boolean, Boolean>>()
    internal val errors = ConcurrentHashMap<String, String>()
    internal val senders = ConcurrentHashMap<String, (String) -> Unit>()
    internal val lines = ConcurrentHashMap<String, String>()

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
            (0 until minOf(saved.length(), 256)).mapNotNull { index ->
                runCatching {
                    val entry = saved.getJSONObject(index)
                    val name = entry.getString("session")
                    require(name.matches(Regex("[a-z0-9._-]{1,64}")))
                    ShellTab(ShellTarget.parse(entry.getJSONObject("target")), name)
                }.getOrNull()
            }
        }.getOrDefault(emptyList())
        val requested = ShellTab(initial, requestedSession)
        require(existing.any { it.id == requested.id } || existing.size < 256) { "Close an existing terminal pane before opening another." }
        val merged = if (existing.any { it.id == requested.id }) {
            existing
        } else {
            (existing + requested).distinctBy { it.id }
        }
        tabs = merged
        activeHost = initial.host
        selectedByHost[initial.host] = requested.id
        selected = requested.id
        minimizedHosts.remove(initial.host)
        minimized = minimizedHosts.values.any { it }
        saveTabs(preferences, merged)
    }

    @Synchronized
    internal fun selectTab(tab: ShellTab) {
        selectedByHost[tab.target.host] = tab.id
        selected = tab.id
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
        val hostCount = tabs.count { it.target.host == target.host }
        if (hostCount >= 16 || tabs.size >= 32) return
        val tab = ShellTab(target, "tab-${System.currentTimeMillis()}")
        val next = tabs + tab
        tabs = next
        selectedByHost[target.host] = tab.id
        selected = tab.id
        saveTabs(context.applicationContext.getSharedPreferences("shell-terminal", Context.MODE_PRIVATE), next)
    }

    @Synchronized
    internal fun closeTab(context: Context?, id: String, terminate: Boolean = true): Boolean {
        val closedTab = tabs.firstOrNull { it.id == id }
        val host = closedTab?.target?.host.orEmpty()
        val next = tabs.filterNot { it.id == id }
        holders.remove(id)?.close(terminate)
        senders.remove(id)
        paths.remove(id)
        errors.remove(id)
        states.remove(id)
        inputStates.remove(id)
        titles.remove(id)
        lines.remove(id)
        if (next.isEmpty()) {
            closeAll(context, terminate = false)
            return true
        }
        tabs = next
        val remainingForHost = next.filter { it.target.host == host }
        if (remainingForHost.isEmpty()) {
            selectedByHost.remove(host)
            minimizedHosts.remove(host)
            minimized = minimizedHosts.values.any { it }
            if (activeHost == host) {
                activeHost = next.first().target.host
            }
            if (next.none { it.id == selected }) {
                selected = next.first().id
            }
            context?.let {
                saveTabs(it.applicationContext.getSharedPreferences("shell-terminal", Context.MODE_PRIVATE), next)
            }
            return true
        }
        if (remainingForHost.none { it.id == selectedByHost[host] }) {
            selectedByHost[host] = remainingForHost.first().id
        }
        if (host == activeHost || next.none { it.id == selected }) {
            selected = selectedByHost[host] ?: remainingForHost.first().id
        }
        context?.let {
            saveTabs(it.applicationContext.getSharedPreferences("shell-terminal", Context.MODE_PRIVATE), next)
        }
        return false
    }

    @Synchronized
    fun minimize(host: String = activeHost) {
        val targetHost = host.ifEmpty { tabs.firstOrNull()?.target?.host.orEmpty() }
        if (targetHost.isNotEmpty() && tabs.any { it.target.host == targetHost }) {
            minimizedHosts[targetHost] = true
            minimized = true
        }
    }

    @Synchronized
    fun closeHost(context: Context?, host: String, terminate: Boolean = true) {
        if (host.isEmpty()) return
        val matching = tabs.filter { it.target.host == host }.map { it.id }
        matching.forEach { closeTab(context, it, terminate) }
        minimizedHosts.remove(host)
        selectedByHost.remove(host)
        minimized = minimizedHosts.values.any { it }
    }

    @Synchronized
    fun closeAll(context: Context? = null, terminate: Boolean = true) {
        val activeHolders = holders.values.toList()
        holders.clear()
        activeHolders.forEach { it.close(terminate) }
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
        activeHost = ""
        selected = ""
        minimized = false
        selectedByHost.clear()
        minimizedHosts.clear()
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
    private fun hostSnapshotMap(host: String, hostTabs: List<ShellTab>): Map<String, Any> {
        val selectedId = selectedByHost[host] ?: selected
        val current = hostTabs.firstOrNull { it.id == selectedId } ?: hostTabs.first()
        val suffix = titles[current.id] ?: current.session
        val countBadge = if (hostTabs.size > 1) " (${hostTabs.size})" else ""
        val title = "${current.target.name} · $suffix$countBadge"
        val stateName = when (states[current.id] ?: ConnectionState.Connecting) {
            ConnectionState.Attached -> "attached"
            ConnectionState.Connecting -> "connecting"
            ConnectionState.Disconnected -> "disconnected"
            ConnectionState.Exited -> "exited"
            ConnectionState.Failed -> "failed"
        }
        val isMinimized = minimizedHosts[host] == true || (minimized && minimizedHosts.isEmpty())
        return mapOf(
            "host" to current.target.host,
            "host_name" to current.target.name,
            "service" to current.target.service,
            "incarnation" to current.target.incarnation,
            "session" to current.session,
            "title" to title,
            "state" to stateName,
            "tab_count" to hostTabs.size,
            "minimized" to isMinimized,
        )
    }

    @Synchronized
    fun snapshots(): List<Map<String, Any>> {
        val list = mutableListOf<Map<String, Any>>()
        tabs.groupBy { it.target.host }.forEach { (host, hostTabs) ->
            if (hostTabs.isNotEmpty()) {
                list.add(hostSnapshotMap(host, hostTabs))
            }
        }
        return list
    }

    private fun mapToJson(map: Map<String, Any>): JSONObject {
        val json = JSONObject()
        map.forEach { (k, v) -> json.put(k, v) }
        return json
    }

    @Synchronized
    fun snapshotsJson(): JSONArray {
        val array = JSONArray()
        snapshots().forEach { array.put(mapToJson(it)) }
        return array
    }

    @Synchronized
    fun snapshotJson(): JSONObject? {
        val all = snapshots()
        if (all.isEmpty()) return null
        val entry = all.firstOrNull { it["minimized"] == true } ?: all.first()
        return mapToJson(entry)
    }

    private fun saveTabs(preferences: android.content.SharedPreferences, currentTabs: List<ShellTab>) {
        preferences.edit().putString(
            "tabs",
            JSONArray().apply { currentTabs.forEach { put(it.json()) } }.toString(),
        ).apply()
    }
}

