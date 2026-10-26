// SPDX-License-Identifier: Apache-2.0
package com.biplabs.wispnet.shell

/** Session-owned retry lifecycle. All calls and scheduled actions run on the main thread. */
internal class ShellReconnectController(
    private val schedule: (Long, () -> Unit) -> (() -> Unit),
    private val connect: () -> Unit,
) {
    private var state = ConnectionState.Connecting
    private var attempts = 0
    private var cancelRetry: (() -> Unit)? = null
    private var closed = false

    fun onState(next: ConnectionState) {
        if (closed) return
        state = next
        cancelRetry?.invoke()
        cancelRetry = null
        if (next == ConnectionState.Attached) attempts = 0
        if (next == ConnectionState.Disconnected) {
            val delay = minOf(30_000L, 1_000L shl minOf(attempts, 5))
            attempts = minOf(attempts + 1, 5)
            cancelRetry = schedule(delay) { retryNow() }
        }
    }

    fun retryNow(explicit: Boolean = false) {
        if (closed || state == ConnectionState.Attached || state == ConnectionState.Connecting) return
        if (!explicit && state != ConnectionState.Disconnected) return
        cancelRetry?.invoke()
        cancelRetry = null
        state = ConnectionState.Connecting
        connect()
    }

    fun close() {
        closed = true
        cancelRetry?.invoke()
        cancelRetry = null
    }
}

// The native API currently returns textual errors. Match its explicit policy errors,
// rather than treating a timeout or unavailable host as a permanent failure.
internal fun shellFailureNeedsAction(message: String): Boolean = listOf(
    "Service no longer permitted", "Pair with this host first", "Invalid shell service or session",
    "Shell hosting is available on desktop computers", "access denied", "permission denied", "access revoked",
).any { message.contains(it, ignoreCase = true) }
