package dev.utter.app

import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import kotlin.concurrent.thread

/**
 * The Rust core, loaded once and kept while the service runs, so a dictation never waits for a
 * load. Its events come back on the main thread, in order.
 */
class Core(private val context: Context, private val onEvent: (Event) -> Unit) {
    @Volatile private var handle = 0L
    private var events: Thread? = null
    private val main = Handler(Looper.getMainLooper())

    val ready: Boolean get() = handle != 0L

    /** Loads off the main thread; [onLoaded] gets null, or why it could not load. */
    fun load(onLoaded: (String?) -> Unit) {
        thread(name = "utter-load") {
            val started = SystemClock.elapsedRealtime()
            val error = try {
                handle = Native.load(config().toString())
                null
            } catch (e: RuntimeException) {
                e.message ?: e.toString()
            }
            Log.i(TAG, "core load: ${error ?: "ok"} in ${SystemClock.elapsedRealtime() - started} ms")
            if (error == null) pollEvents()
            main.post { onLoaded(error) }
        }
    }

    fun touch(down: Boolean) = ifReady { Native.touch(it, down, SystemClock.uptimeMillis().toInt()) }

    fun cancel() = ifReady { Native.cancel(it) }

    fun beforeCursor(text: String) = ifReady { Native.beforeCursor(it, text) }

    fun pushAudio(pcm: ShortArray, length: Int) = ifReady { Native.pushAudio(it, pcm, length) }

    /** Frees the core once its event thread has stopped using it. */
    fun close() {
        val h = handle
        handle = 0
        events?.join()
        events = null
        if (h != 0L) Native.free(h)
    }

    private inline fun ifReady(call: (Long) -> Unit) {
        val h = handle
        if (h != 0L) call(h)
    }

    private fun pollEvents() {
        events = thread(name = "utter-events") {
            while (true) {
                val h = handle
                if (h == 0L) break
                val json = Native.nextEvent(h, 250) ?: continue
                val event = Event.parse(json)
                main.post { onEvent(event) }
            }
        }
    }

    private fun config(): JSONObject {
        val models = Models(context)
        return JSONObject()
            .put("data_dir", context.filesDir.path)
            .put("native_dir", context.applicationInfo.nativeLibraryDir)
            .put("speech_dir", models.speech.path)
            .put("vad", models.vad.path)
            // The emulator's GPU is a software renderer: the encoder runs on its CPU.
            .put("cpu_encoder", isEmulator)
            .put("cleanup_model", models.cleanup?.path ?: JSONObject.NULL)
            .put("dictionary", JSONArray())
    }

    companion object {
        private const val TAG = "utter"
        val isEmulator: Boolean get() = Build.HARDWARE == "ranchu" || Build.HARDWARE == "goldfish"
    }
}
