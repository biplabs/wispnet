// SPDX-License-Identifier: Apache-2.0
package com.biplabs.wispnet.shell

import org.json.JSONObject

data class ShellTarget(val host: String, val service: String, val incarnation: String, val name: String) {
    fun valid(): Boolean = host.matches(Regex("[0-9a-f]{64}")) &&
        incarnation.matches(Regex("[0-9a-f]{64}")) &&
        service.matches(Regex("[a-z0-9][a-z0-9._-]{0,63}"))

    fun json(): JSONObject = JSONObject().put("host", host).put("service", service)
        .put("incarnation", incarnation).put("name", name)

    companion object {
        fun parse(value: JSONObject): ShellTarget = ShellTarget(
            value.getString("host"), value.getString("service"),
            value.getString("incarnation"), value.optString("name", value.optString("host_name", "Computer")),
        ).also { require(it.valid()) { "Invalid shell service" } }
    }
}

enum class TerminalInputMode { Auto, Sync, Predictive, Line;
    fun effective(latencyMs: Int?): TerminalInputMode = if (this != Auto) this else when {
        latencyMs == null -> Predictive
        latencyMs <= 80 -> Sync
        latencyMs <= 220 -> Predictive
        else -> Line
    }
    fun resolve(latencyMs: Int?, echo: Boolean?, canonical: Boolean?): TerminalInputMode =
        if (echo != true || canonical != true) Sync else effective(latencyMs)
}

enum class ConnectionState { Connecting, Attached, Disconnected, Exited }

interface WispTerminalConnection {
    fun connect()
    fun sendInput(data: String)
    fun sendBytes(bytes: ByteArray)
    fun resize(cols: Int, rows: Int)
    fun close()
}
