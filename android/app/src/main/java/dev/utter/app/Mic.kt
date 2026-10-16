package dev.utter.app

import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.SystemClock
import android.util.Log
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.concurrent.thread
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sqrt

/**
 * The microphone, open only while it is needed: 16 kHz mono 16-bit, in 30 ms chunks to
 * [onAudio], with the chunk's level (0 to 1) for the bubble's ring.
 *
 * Development builds read `test-audio.wav` (16 kHz mono 16-bit) from the files folder instead
 * when it is there, at speaking pace, then silence: the emulator's microphone stays off, as the
 * desktop's test audio does (utter_pipeline::apply_test_audio).
 */
class Mic(private val context: Context, private val onAudio: (ShortArray, Int, Float) -> Unit) {
    @Volatile private var running = false
    private var worker: Thread? = null

    val isOpen: Boolean get() = running

    fun open() {
        if (running) return
        running = true
        worker = thread(name = "utter-mic") {
            val test = testAudio()
            if (test != null) replay(test) else record()
        }
    }

    /** Closes it; returns once the last chunk is handed over (at most one chunk's time). */
    fun close() {
        running = false
        worker?.join()
        worker = null
    }

    @SuppressLint("MissingPermission") // asked for in the first run; without it read() fails and nothing is heard
    private fun record() {
        val min = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val recorder = try {
            AudioRecord(MediaRecorder.AudioSource.VOICE_RECOGNITION, RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, max(min, RATE))
        } catch (e: RuntimeException) {
            Log.w(TAG, "microphone: $e")
            return
        }
        recorder.startRecording()
        val buffer = ShortArray(CHUNK)
        while (running) {
            val n = recorder.read(buffer, 0, buffer.size)
            if (n > 0) onAudio(buffer.copyOf(n), n, level(buffer, n))
        }
        recorder.stop()
        recorder.release()
    }

    private fun replay(samples: ShortArray) {
        val start = SystemClock.uptimeMillis()
        var at = 0
        while (running) {
            val chunk = if (at < samples.size) samples.copyOfRange(at, min(at + CHUNK, samples.size)) else ShortArray(CHUNK)
            onAudio(chunk, chunk.size, level(chunk, chunk.size))
            at += CHUNK
            val wait = start + at * 1000L / RATE - SystemClock.uptimeMillis()
            if (wait > 0) Thread.sleep(wait)
        }
    }

    private fun testAudio(): ShortArray? {
        if (!BuildConfig.DEBUG) return null
        val file = File(context.filesDir, "test-audio.wav")
        if (!file.isFile) return null
        val bytes = ByteBuffer.wrap(file.readBytes()).order(ByteOrder.LITTLE_ENDIAN)
        // RIFF chunks after the 12-byte header: find "data".
        var at = 12
        while (at + 8 <= bytes.limit()) {
            val id = String(ByteArray(4) { bytes.get(at + it) })
            val size = bytes.getInt(at + 4)
            if (id == "data") {
                val samples = ShortArray(min(size, bytes.limit() - at - 8) / 2)
                bytes.position(at + 8)
                bytes.asShortBuffer().get(samples)
                Log.i(TAG, "test audio: ${samples.size / RATE.toFloat()} s from ${file.path}")
                return samples
            }
            at += 8 + size + (size and 1)
        }
        return null
    }

    private fun level(samples: ShortArray, n: Int): Float {
        var sum = 0.0
        for (i in 0 until n) sum += samples[i].toDouble() * samples[i]
        return min(1f, (sqrt(sum / max(n, 1)) / 6000.0).toFloat())
    }

    companion object {
        private const val TAG = "utter"
        const val RATE = 16_000
        /** 30 ms. */
        const val CHUNK = 480
    }
}
