package dev.utter.app

import android.Manifest
import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Typeface
import android.os.Bundle
import android.provider.Settings
import android.text.format.Formatter
import android.util.TypedValue
import android.view.Gravity
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView

/**
 * The first run (docs/ux/android.md, "First run"): one screen at a time, each with a single
 * action, then the place to try it. Opened again later, it goes straight to the first step not
 * done.
 */
class MainActivity : Activity() {
    private enum class Step { Intro, Microphone, Accessibility, Models, TryIt }

    private val prefs by lazy { getSharedPreferences("first-run", MODE_PRIVATE) }

    override fun onResume() {
        super.onResume()
        show(nextStep())
    }

    private fun nextStep(): Step = when {
        !prefs.getBoolean("intro", false) -> Step.Intro
        checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED -> Step.Microphone
        !serviceEnabled() -> Step.Accessibility
        Models(this).missing().isNotEmpty() -> Step.Models
        else -> Step.TryIt
    }

    private fun serviceEnabled(): Boolean {
        val enabled = Settings.Secure.getString(contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES) ?: return false
        val ours = ComponentName(this, UtterService::class.java)
        return enabled.split(':').any { ComponentName.unflattenFromString(it) == ours }
    }

    private fun show(step: Step) {
        when (step) {
            Step.Intro -> screen(
                "Dictate into any app",
                "Hold the bubble next to a text field, speak, and let go: the cleaned-up text is in the field. " +
                    "Speech and cleanup run on this phone; nothing leaves it.",
                "Continue",
            ) {
                prefs.edit().putBoolean("intro", true).apply()
                show(nextStep())
            }
            Step.Microphone -> screen(
                "Microphone",
                "Utter listens only while you hold the bubble, or after you double-tap it until you tap again.",
                "Allow the microphone",
            ) { requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 1) }
            Step.Accessibility -> screen(
                "Accessibility",
                getString(R.string.accessibility_description) + "\n\nTurn on Utter under Accessibility, then come back.",
                "Open Accessibility settings",
            ) { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) }
            Step.Models -> {
                val models = Models(this)
                val have = models.dir.walkTopDown().filter { it.isFile }.sumOf { it.length() }
                screen(
                    "Models",
                    "Speech and cleanup models, about 2 GB. Missing: ${models.missing().joinToString()}" +
                        " (${Formatter.formatShortFileSize(this, have)} installed). " +
                        "Downloading them is not built yet; development builds get them through adb.",
                    "Check again",
                ) { show(nextStep()) }
            }
            Step.TryIt -> tryIt()
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        show(nextStep())
    }

    private fun dp(v: Float) = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, resources.displayMetrics).toInt()

    private fun column() = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        gravity = Gravity.CENTER_VERTICAL
        setPadding(dp(24f), dp(48f), dp(24f), dp(24f))
    }

    private fun title(text: String) = TextView(this).apply {
        this.text = text
        textSize = 26f
        typeface = Typeface.DEFAULT_BOLD
    }

    private fun body(text: String) = TextView(this).apply {
        this.text = text
        textSize = 17f
        setPadding(0, dp(16f), 0, dp(24f))
    }

    private fun screen(title: String, body: String, action: String, onAction: () -> Unit) {
        setContentView(column().apply {
            addView(title(title))
            addView(body(body))
            addView(Button(this@MainActivity).apply {
                text = action
                setOnClickListener { onAction() }
            })
        })
    }

    private fun tryIt() {
        setContentView(column().apply {
            gravity = Gravity.TOP
            addView(title("Try it"))
            addView(body("Tap the field, then hold the bubble at the right edge and speak. Let go when you are done; " +
                "double-tap it to keep listening without holding."))
            addView(EditText(this@MainActivity).apply {
                hint = "Dictate here"
                minLines = 3
                gravity = Gravity.TOP
            })
        })
    }
}
