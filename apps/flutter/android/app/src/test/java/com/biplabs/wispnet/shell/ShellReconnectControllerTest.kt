// SPDX-License-Identifier: Apache-2.0
package com.biplabs.wispnet.shell

import org.junit.Assert.*
import org.junit.Test

class ShellReconnectControllerTest {
    private class Harness {
        var connects = 0
        var task: (() -> Unit)? = null
        val delays = mutableListOf<Long>()
        val controller = ShellReconnectController(
            schedule = { delay, action ->
                assertNull("Only one retry may be scheduled", task)
                delays += delay
                task = action
                val cancel: () -> Unit = { task = null }
                cancel
            },
            connect = { connects++ },
        )
        fun tick() { val action = task!!; task = null; action() }
    }

    @Test fun retriesBeyondThreeAttemptsAndCapsDelay() {
        val h = Harness()
        repeat(9) {
            h.controller.onState(ConnectionState.Disconnected)
            h.tick()
        }
        assertEquals(9, h.connects)
        assertEquals(listOf(1000L, 2000L, 4000L, 8000L, 16000L, 30000L, 30000L, 30000L, 30000L), h.delays)
    }

    @Test fun networkOrResumeRetriesImmediatelyWithoutOverlappingAttempts() {
        val h = Harness()
        h.controller.onState(ConnectionState.Disconnected)
        h.controller.retryNow()
        assertNull(h.task)
        h.controller.retryNow()
        assertEquals(1, h.connects)
        h.controller.onState(ConnectionState.Attached)
        h.controller.retryNow()
        assertEquals(1, h.connects)
        h.controller.onState(ConnectionState.Disconnected)
        assertEquals(1000L, h.delays.last())
    }

    @Test fun exitAndPolicyFailureNeedExplicitActionAndCloseCancelsRetries() {
        for (state in listOf(ConnectionState.Exited, ConnectionState.Failed)) {
            val h = Harness()
            h.controller.onState(ConnectionState.Disconnected)
            h.controller.onState(state)
            assertNull(h.task)
            h.controller.retryNow()
            assertEquals(0, h.connects)
            h.controller.retryNow(explicit = true)
            assertEquals(1, h.connects)
            h.controller.onState(ConnectionState.Disconnected)
            h.controller.close()
            assertNull(h.task)
            h.controller.retryNow(explicit = true)
            h.controller.onState(ConnectionState.Disconnected)
            assertNull(h.task)
            assertEquals(1, h.connects)
        }
    }

    @Test fun distinguishesPolicyErrorsFromTemporaryNetworkFailures() {
        assertTrue(shellFailureNeedsAction("Service no longer permitted"))
        assertTrue(shellFailureNeedsAction("Pair with this host first"))
        assertTrue(shellFailureNeedsAction("Permission denied"))
        assertFalse(shellFailureNeedsAction("Shell connection closed"))
        assertFalse(shellFailureNeedsAction("connection timed out"))
    }
}
