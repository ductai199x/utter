package dev.utter.app

import android.accessibilityservice.AccessibilityService
import android.graphics.Rect
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
import kotlin.concurrent.thread

/**
 * Utter on Android: the bubble beside the focused text field, dictation into it. Loads the core
 * when the service starts and keeps it while the service runs.
 *
 * The microphone is open while the bubble is held or the core is listening (the two overlap:
 * a double tap's first release cancels while the second press is already down).
 */
class UtterService : AccessibilityService(), BubbleTouches {
    private lateinit var core: Core
    private lateinit var mic: Mic
    private lateinit var overlay: Overlay
    private var pressed = false
    private var listening = false
    private var finishing = false
    private val main = Handler(Looper.getMainLooper())

    override fun onServiceConnected() {
        overlay = Overlay(this, this)
        mic = Mic(this) { pcm, n, level ->
            core.pushAudio(pcm, n)
            main.post { overlay.level(level) }
        }
        core = Core(this, ::onEvent)
        thread(name = "utter-start") {
            val missing = Models(this).missing()
            if (missing.isNotEmpty()) {
                Log.w(TAG, "models missing: $missing")
                return@thread
            }
            core.load { error ->
                if (error != null) Log.e(TAG, "the core did not load: $error") else updateBubble()
            }
        }
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent) {
        updateBubble()
    }

    override fun onInterrupt() {}

    override fun onDestroy() {
        if (::mic.isInitialized) mic.close()
        if (::overlay.isInitialized) overlay.hideBubble()
        if (::core.isInitialized) core.close()
        super.onDestroy()
    }

    /** The bubble is there while a field dictation may go into has the focus and the keyboard is up. */
    private fun updateBubble() {
        if (!::core.isInitialized || !core.ready) return
        val keyboard = windows.firstOrNull { it.type == AccessibilityWindowInfo.TYPE_INPUT_METHOD }
        val node = focusedField()
        if (keyboard != null && node != null) {
            val bounds = Rect().also { keyboard.getBoundsInScreen(it) }
            overlay.showBubble(bounds.top)
        } else if (!listening && !finishing && !pressed) {
            overlay.hideBubble()
        }
    }

    private fun focusedField(): AccessibilityNodeInfo? =
        findFocus(AccessibilityNodeInfo.FOCUS_INPUT)?.takeIf { Field.accepts(it) }

    override fun pressed() {
        pressed = true
        updateMic()
        core.touch(true)
    }

    override fun released() {
        pressed = false
        core.touch(false)
        updateMic()
    }

    override fun draggedOff() {
        core.cancel()
    }

    private fun updateMic() {
        if (pressed || listening) mic.open() else mic.close()
    }

    private fun onEvent(event: Event) {
        when (event) {
            is Event.Listening -> {
                listening = true
                updateMic()
                overlay.state(if (event.handsFree) BubbleState.HandsFree else BubbleState.Listening)
                focusedField()?.let { core.beforeCursor(Field(it).beforeCursor()) }
            }
            is Event.Words -> overlay.words(event.committed, event.preview)
            Event.Finishing -> {
                listening = false
                finishing = true
                updateMic()
                overlay.state(BubbleState.Finishing)
            }
            is Event.Final -> {
                finishing = false
                if (event.insert.isEmpty()) {
                    overlay.message("Didn't hear anything")
                } else {
                    when (insert(this, focusedField()?.let(::Field), event.insert)) {
                        Inserted.SetText, Inserted.Pasted -> overlay.state(BubbleState.Done)
                        Inserted.Copied -> overlay.message("Couldn't type here: copied instead")
                    }
                }
                Log.i(TAG, "dictation: ready ${event.totalMs.toInt()} ms after release")
                updateBubble()
            }
            Event.Cancelled -> {
                listening = false
                updateMic()
                overlay.state(BubbleState.Idle)
                updateBubble()
            }
            is Event.Error -> {
                listening = false
                finishing = false
                updateMic()
                Log.e(TAG, "dictation: ${event.message}")
                overlay.message("Something went wrong: ${event.message}")
                updateBubble()
            }
        }
    }

    companion object {
        private const val TAG = "utter"
    }
}
