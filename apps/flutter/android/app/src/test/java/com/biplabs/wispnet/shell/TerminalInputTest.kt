// SPDX-License-Identifier: AGPL-3.0-only
package com.biplabs.wispnet.shell

import org.junit.Assert.assertEquals
import org.junit.Test

class TerminalInputTest {
    @Test fun composingAndCommittingDoesNotDuplicateInput() {
        val composition = TerminalComposition()
        assertEquals(listOf("hel"), composition.update("hel"))
        assertEquals(listOf("lo"), composition.update("hello"))
        assertEquals(listOf("\u007f", "\u007f", "p"), composition.update("help"))
        assertEquals(emptyList<String>(), composition.update("help"))
        composition.finish()
        assertEquals(listOf("help"), composition.update("help"))
    }

    @Test fun unicodeCompositionDeletesCharactersRatherThanSurrogateHalves() {
        val composition = TerminalComposition()
        assertEquals(listOf("a😀"), composition.update("a😀"))
        assertEquals(listOf("\u007f", "😁"), composition.update("a😁"))
        composition.deleted()
        assertEquals(emptyList<String>(), composition.update("a"))
    }

    @Test fun autoModeUsesTheMobileLatencyThresholds() {
        val auto = TerminalInputMode.Auto
        assertEquals(TerminalInputMode.Sync, auto.resolve(80, true, true))
        assertEquals(TerminalInputMode.Predictive, auto.resolve(81, true, true))
        assertEquals(TerminalInputMode.Predictive, auto.resolve(220, true, true))
        assertEquals(TerminalInputMode.Line, auto.resolve(221, true, true))
        assertEquals(TerminalInputMode.Predictive, auto.resolve(null, true, true))
    }

    @Test fun passwordAndRawModeInputNeverUseLocalEchoOrCommandComposer() {
        TerminalInputMode.entries.forEach { mode ->
            assertEquals(TerminalInputMode.Sync, mode.resolve(500, false, true))
            assertEquals(TerminalInputMode.Sync, mode.resolve(500, true, false))
            assertEquals(TerminalInputMode.Sync, mode.resolve(500, null, null))
        }
        assertEquals(TerminalInputMode.Line, TerminalInputMode.Line.resolve(5, true, true))
    }

    @Test fun workspaceRetainsTabsWhenMinimizedAndClearsOnClose() {
        ShellWorkspace.closeAll()
        val target = ShellTarget("a".repeat(64), "shell", "b".repeat(64), "j-strix")
        val tab = ShellTab(target, "main")
        ShellWorkspace.tabs = listOf(tab)
        ShellWorkspace.selected = tab.id
        ShellWorkspace.titles[tab.id] = "wispnet"
        ShellWorkspace.states[tab.id] = ConnectionState.Attached
        ShellWorkspace.minimize()
        assertEquals(true, ShellWorkspace.minimized)
        assertEquals(listOf(tab), ShellWorkspace.tabs)
        ShellWorkspace.closeAll()
        assertEquals(false, ShellWorkspace.minimized)
        assertEquals(emptyList<ShellTab>(), ShellWorkspace.tabs)
        assertEquals(true, ShellWorkspace.titles.isEmpty())
    }

    @Test fun workspaceIsolatesHostsAndMinimizesPerHost() {
        ShellWorkspace.closeAll()
        val hostA = ShellTarget("a".repeat(64), "shell", "b".repeat(64), "j-strix")
        val hostB = ShellTarget("c".repeat(64), "shell", "d".repeat(64), "j-desktop")
        val tabA = ShellTab(hostA, "main")
        val tabB = ShellTab(hostB, "main")
        ShellWorkspace.tabs = listOf(tabA, tabB)
        ShellWorkspace.selectTab(tabA)
        ShellWorkspace.selectTab(tabB)
        ShellWorkspace.minimize(hostA.host)
        ShellWorkspace.minimize(hostB.host)
        val snapshots = ShellWorkspace.snapshots()
        assertEquals(2, snapshots.size)
        assertEquals("j-strix · main", snapshots[0]["title"])
        assertEquals(true, snapshots[0]["minimized"])
        assertEquals("j-desktop · main", snapshots[1]["title"])
        assertEquals(true, snapshots[1]["minimized"])
        ShellWorkspace.closeHost(null, hostA.host, terminate = true)
        assertEquals(listOf(tabB), ShellWorkspace.tabs)
        assertEquals(true, ShellWorkspace.minimized)
        assertEquals(1, ShellWorkspace.snapshots().size)
        ShellWorkspace.closeHost(null, hostB.host, terminate = true)
        assertEquals(false, ShellWorkspace.minimized)
        assertEquals(emptyList<ShellTab>(), ShellWorkspace.tabs)
    }
}
