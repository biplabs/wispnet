// SPDX-License-Identifier: AGPL-3.0-only
// IME composition reconciliation adapted from Biplabs WispShell.
package com.biplabs.wispnet.shell

/** Translate IME replacement/composition updates into remote terminal edits. */
internal class TerminalComposition {
    private var composing = ""

    fun update(value: String): List<String> {
        val previous = composing.codePoints().toArray()
        val next = value.codePoints().toArray()
        var common = 0
        while (common < minOf(previous.size, next.size) && previous[common] == next[common]) common++
        val edits = MutableList(previous.size - common) { "\u007f" }
        if (common < next.size) edits.add(String(next, common, next.size - common))
        composing = value
        return edits
    }

    fun finish() { composing = "" }
    fun deleted() {
        if (composing.isNotEmpty()) composing = composing.substring(0, composing.offsetByCodePoints(composing.length, -1))
    }
}
