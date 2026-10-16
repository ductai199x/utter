package dev.utter.app

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.Bundle
import android.text.InputType
import android.view.accessibility.AccessibilityNodeInfo
import kotlin.math.max
import kotlin.math.min

/** How a dictation went in (docs/ux/android.md, "Inserting"). */
enum class Inserted { SetText, Pasted, Copied }

/** The focused field's text and selection, as far as the app reports them. */
class Field(val node: AccessibilityNodeInfo) {
    /** The text, without a hint shown in an empty field. */
    val text: String = if (node.isShowingHintText) "" else node.text?.toString() ?: ""
    private val start = node.textSelectionStart
    private val end = node.textSelectionEnd
    /** The selection; at the end when the app does not say where the cursor is. */
    val selection: IntRange =
        if (start < 0 || end < 0 || max(start, end) > text.length) text.length..text.length else min(start, end)..max(start, end)

    /** Up to [n] characters before the cursor, for spacing. */
    fun beforeCursor(n: Int = 64): String = text.substring(max(0, selection.first - n), selection.first)

    companion object {
        /** A field dictation may go into: editable, and not a password, PIN or other sensitive field. */
        fun accepts(node: AccessibilityNodeInfo): Boolean {
            if (!node.isEditable || node.isPassword) return false
            val variation = node.inputType and InputType.TYPE_MASK_VARIATION
            val klass = node.inputType and InputType.TYPE_MASK_CLASS
            val password = when (klass) {
                InputType.TYPE_CLASS_TEXT -> variation == InputType.TYPE_TEXT_VARIATION_PASSWORD ||
                    variation == InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD ||
                    variation == InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD
                InputType.TYPE_CLASS_NUMBER -> variation == InputType.TYPE_NUMBER_VARIATION_PASSWORD
                else -> false
            }
            if (password) return false
            return !(Build.VERSION.SDK_INT >= 34 && node.isAccessibilityDataSensitive)
        }
    }
}

/**
 * Puts [insert] into the field at the cursor, replacing the selection, and the cursor after it:
 * through the field's set-text action; when the app refuses it, by pasting through the
 * clipboard and putting back what was there; when both fail, the text is left on the
 * clipboard.
 */
fun insert(context: Context, field: Field?, insert: String): Inserted {
    val clipboard = context.getSystemService(ClipboardManager::class.java)
    if (field != null) {
        val node = field.node
        val at = field.selection
        val text = field.text.substring(0, at.first) + insert + field.text.substring(at.last)
        val args = Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text) }
        if (node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args)) {
            val cursor = at.first + insert.length
            val selection = Bundle().apply {
                putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, cursor)
                putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, cursor)
            }
            node.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, selection)
            return Inserted.SetText
        }
        // Reading the clipboard is refused to a service in the background on recent Android:
        // then there is nothing to put back.
        val previous = runCatching { clipboard.primaryClip }.getOrNull()
        clipboard.setPrimaryClip(ClipData.newPlainText("Utter", insert))
        if (node.performAction(AccessibilityNodeInfo.ACTION_PASTE)) {
            if (previous != null) node.refresh().also { clipboard.setPrimaryClip(previous) }
            return Inserted.Pasted
        }
        return Inserted.Copied
    }
    clipboard.setPrimaryClip(ClipData.newPlainText("Utter", insert))
    return Inserted.Copied
}
