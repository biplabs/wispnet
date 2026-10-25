// SPDX-License-Identifier: Apache-2.0
package com.biplabs.wispnet.shell

import com.termux.terminal.TerminalEmulator
import com.termux.terminal.TerminalOutput
import com.termux.terminal.TerminalSessionClient
import java.io.ByteArrayOutputStream
import java.lang.reflect.Proxy
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Interactive shells query the terminal before drawing their first prompt. */
class TerminalQueryTest {
    @Test fun emulatorAnswersShellStatusCursorDeviceAndBackgroundQueries() {
        val replies = ByteArrayOutputStream()
        val output = object : TerminalOutput() {
            override fun write(data: ByteArray, offset: Int, count: Int) { replies.write(data, offset, count) }
            override fun titleChanged(oldTitle: String?, newTitle: String?) = Unit
            override fun onCopyTextToClipboard(text: String?) = Unit
            override fun onPasteTextFromClipboard() = Unit
            override fun onBell() = Unit
            override fun onColorsChanged() = Unit
        }
        val client = Proxy.newProxyInstance(TerminalSessionClient::class.java.classLoader,
            arrayOf(TerminalSessionClient::class.java)) { _, _, _ -> null } as TerminalSessionClient
        val emulator = TerminalEmulator(output, 80, 24, 2000, client)
        fun query(value: String): String {
            replies.reset()
            val bytes = value.toByteArray(Charsets.UTF_8)
            emulator.append(bytes, bytes.size)
            return replies.toString("UTF-8")
        }
        assertEquals("\u001b[0n", query("\u001b[5n"))
        assertEquals("\u001b[1;1R", query("\u001b[6n"))
        val device = query("\u001b[c")
        assertTrue(device.startsWith("\u001b[?"))
        assertTrue(device.endsWith("c"))
        assertTrue(query("\u001b]11;?\u0007").startsWith("\u001b]11;rgb:"))
    }
}
