package dev.utter.app

/** The Rust core's calls (crates/utter-mobile/src/jni.rs). A loaded core is a handle. */
object Native {
    init {
        System.loadLibrary("utter_mobile")
    }

    /** Loads the models (seconds); throws with the reason when it cannot. */
    @JvmStatic external fun load(configJson: String): Long
    @JvmStatic external fun touch(handle: Long, down: Boolean, timeMs: Int)
    @JvmStatic external fun cancel(handle: Long)
    @JvmStatic external fun beforeCursor(handle: Long, text: String)
    @JvmStatic external fun pushAudio(handle: Long, pcm: ShortArray, length: Int)
    /** The next event as JSON, or null after [timeoutMs] without one. */
    @JvmStatic external fun nextEvent(handle: Long, timeoutMs: Int): String?
    @JvmStatic external fun free(handle: Long)
}
