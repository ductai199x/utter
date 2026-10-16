package dev.utter.app

import android.animation.ValueAnimator
import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.PixelFormat
import android.graphics.RectF
import android.graphics.drawable.GradientDrawable
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.style.ForegroundColorSpan
import android.util.TypedValue
import android.view.Gravity
import android.view.HapticFeedbackConstants
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.view.animation.LinearInterpolator
import android.widget.TextView
import kotlin.math.hypot

/** What the bubble shows (docs/ux/android.md, "Dictating"). */
enum class BubbleState { Idle, Listening, HandsFree, Finishing, Done }

/** The bubble's touches, as the core needs them. */
interface BubbleTouches {
    fun pressed()
    fun released()
    /** The finger left the bubble while holding it. */
    fun draggedOff()
}

/**
 * The bubble at the screen's right edge and the pill above the keyboard, as accessibility
 * overlays: no "draw over other apps" permission, never focusable, so the field keeps the
 * focus and the keyboard stays.
 */
class Overlay(private val context: Context, private val touches: BubbleTouches) {
    private val windows = context.getSystemService(WindowManager::class.java)
    private val bubble = BubbleView(context, touches)
    private val pill = PillView(context)
    private var bubbleShown = false
    private var pillShown = false
    private var keyboardTop = 0
    private val hidePill = Runnable { setPillShown(false) }

    private fun dp(v: Float) = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, context.resources.displayMetrics)

    private val screen get() = windows.currentWindowMetrics.bounds

    /** Shows the bubble at 60% of the space above the keyboard, whose top is [keyboardTop]. */
    fun showBubble(keyboardTop: Int) {
        this.keyboardTop = keyboardTop
        val size = dp(BubbleView.WINDOW_DP).toInt()
        val params = bubbleParams(size).apply {
            x = screen.width() - size - dp(4f).toInt()
            y = (keyboardTop * 0.6f - size / 2f).toInt().coerceAtLeast(0)
        }
        if (bubbleShown) {
            windows.updateViewLayout(bubble, params)
        } else {
            bubble.alpha = 0f
            windows.addView(bubble, params)
            bubbleShown = true
        }
        bubble.animate().alpha(1f).setDuration(150).start()
    }

    fun hideBubble() {
        if (!bubbleShown) return
        bubbleShown = false
        bubble.animate().alpha(0f).setDuration(150).withEndAction { windows.removeView(bubble) }.start()
        setPillShown(false)
    }

    val isBubbleShown: Boolean get() = bubbleShown

    fun state(state: BubbleState) {
        bubble.state = state
        when (state) {
            BubbleState.Listening, BubbleState.HandsFree -> {
                bubble.performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
                pill.removeCallbacks(hidePill)
                pill.words("", "")
                setPillShown(true)
            }
            BubbleState.Finishing -> pill.dim()
            BubbleState.Done -> {
                bubble.performHapticFeedback(HapticFeedbackConstants.CONFIRM)
                bubble.postDelayed({ if (bubble.state == BubbleState.Done) bubble.state = BubbleState.Idle }, 400)
                setPillShown(false)
            }
            BubbleState.Idle -> setPillShown(false)
        }
    }

    fun level(level: Float) {
        bubble.level = level
    }

    fun words(committed: String, preview: String) = pill.words(committed, preview)

    /** One line in the pill for 2.5 s, the bubble back to idle. */
    fun message(text: String) {
        bubble.state = BubbleState.Idle
        pill.message(text)
        setPillShown(true)
        pill.removeCallbacks(hidePill)
        pill.postDelayed(hidePill, 2500)
    }

    private fun setPillShown(shown: Boolean) {
        if (shown == pillShown) return
        pillShown = shown
        if (shown) {
            val params = WindowManager.LayoutParams(
                screen.width() - 2 * dp(16f).toInt(),
                WindowManager.LayoutParams.WRAP_CONTENT,
                WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE,
                PixelFormat.TRANSLUCENT,
            ).apply {
                gravity = Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
                y = screen.height() - keyboardTop + dp(8f).toInt()
            }
            windows.addView(pill, params)
        } else {
            windows.removeView(pill)
        }
    }

    private fun bubbleParams(size: Int) = WindowManager.LayoutParams(
        size,
        size,
        WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
        WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL or
            WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
        PixelFormat.TRANSLUCENT,
    ).apply { gravity = Gravity.TOP or Gravity.START }
}

/** The bubble: Utter's mark, a ring that follows the voice while listening. */
@SuppressLint("ViewConstructor")
private class BubbleView(context: Context, private val touches: BubbleTouches) : View(context) {
    var state = BubbleState.Idle
        set(value) {
            field = value
            if (value == BubbleState.Finishing) sweep.start() else sweep.cancel()
            invalidate()
        }
    var level = 0f
        set(value) {
            field = field * 0.6f + value * 0.4f
            invalidate()
        }

    private val density = resources.displayMetrics.density
    private val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.rgb(31, 31, 36) }
    private val mark = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.WHITE }
    private val ring = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeWidth = 3 * density
        color = Color.rgb(120, 170, 255)
        strokeCap = Paint.Cap.ROUND
    }
    private val dot = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.rgb(230, 60, 60) }
    private val check = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        strokeWidth = 3 * density
        color = Color.WHITE
        strokeCap = Paint.Cap.ROUND
    }
    private var sweepAngle = 0f
    private val sweep = ValueAnimator.ofFloat(0f, 360f).apply {
        duration = 1200
        repeatCount = ValueAnimator.INFINITE
        interpolator = LinearInterpolator()
        addUpdateListener {
            sweepAngle = it.animatedValue as Float
            invalidate()
        }
    }
    private var pressedDown = false
    private var draggedOff = false

    override fun onDraw(canvas: Canvas) {
        val cx = width / 2f
        val cy = height / 2f
        val listening = state == BubbleState.Listening || state == BubbleState.HandsFree
        val radius = (if (listening || state == BubbleState.Finishing) 28f else 24f) * density
        fill.alpha = if (state == BubbleState.Idle) 178 else 255
        canvas.drawCircle(cx, cy, radius, fill)
        if (state == BubbleState.Done) {
            val s = radius * 0.4f
            canvas.drawLine(cx - s, cy, cx - s * 0.2f, cy + s * 0.7f, check)
            canvas.drawLine(cx - s * 0.2f, cy + s * 0.7f, cx + s, cy - s * 0.6f, check)
        } else {
            mark.alpha = fill.alpha
            val bar = 2f * density
            val heights = floatArrayOf(5f, 11f, 7f)
            for (i in 0..2) {
                val x = cx + (i - 1) * 8f * density
                val h = heights[i] * density
                canvas.drawRect(x - bar, cy - h, x + bar, cy + h, mark)
            }
        }
        val ringRadius = radius + 2.5f * density
        when {
            listening -> {
                ring.alpha = (90 + 165 * level).toInt().coerceIn(0, 255)
                canvas.drawCircle(cx, cy, ringRadius + 2f * density * level, ring)
            }
            state == BubbleState.Finishing -> {
                ring.alpha = 200
                val box = RectF(cx - ringRadius, cy - ringRadius, cx + ringRadius, cy + ringRadius)
                canvas.drawArc(box, sweepAngle, 90f, false, ring)
            }
        }
        if (state == BubbleState.HandsFree) canvas.drawCircle(cx + radius * 0.7f, cy - radius * 0.7f, 4f * density, dot)
    }

    @SuppressLint("ClickableViewAccessibility") // dictation by holding; there is no click
    override fun onTouchEvent(e: MotionEvent): Boolean {
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                pressedDown = true
                draggedOff = false
                touches.pressed()
            }
            MotionEvent.ACTION_MOVE -> {
                val away = hypot(e.x - width / 2f, e.y - height / 2f) > width
                if (pressedDown && !draggedOff && away) {
                    draggedOff = true
                    touches.draggedOff()
                }
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                if (pressedDown) touches.released()
                pressedDown = false
            }
        }
        return true
    }

    companion object {
        /** The window: the 56 dp listening bubble with its ring. */
        const val WINDOW_DP = 72f
    }
}

/** The pill: the words so far, those not cleaned yet dimmed. */
private class PillView(context: Context) : TextView(context) {
    private var dimmed = false

    init {
        val density = resources.displayMetrics.density
        setTextColor(Color.WHITE)
        textSize = 16f
        maxLines = 4
        setPadding((16 * density).toInt(), (10 * density).toInt(), (16 * density).toInt(), (10 * density).toInt())
        background = GradientDrawable().apply {
            setColor(Color.argb(235, 31, 31, 36))
            cornerRadius = 20 * density
        }
    }

    fun words(committed: String, preview: String) {
        dimmed = false
        alpha = 1f
        val text = SpannableStringBuilder(committed)
        if (preview.isNotEmpty()) {
            if (text.isNotEmpty()) text.append(' ')
            val start = text.length
            text.append(preview)
            text.setSpan(ForegroundColorSpan(Color.argb(150, 255, 255, 255)), start, text.length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        }
        setText(if (text.isEmpty()) "Listening…" else text)
    }

    fun dim() {
        dimmed = true
        alpha = 0.6f
    }

    fun message(text: String) {
        alpha = 1f
        setText(text)
    }
}
