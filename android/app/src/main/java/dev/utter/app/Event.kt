package dev.utter.app

import org.json.JSONObject

/** What the core says happened (utter_mobile::Event). */
sealed interface Event {
    data class Listening(val handsFree: Boolean) : Event
    /** The words so far: cleaned up to the last pause, then heard since (provisional). */
    data class Words(val committed: String, val preview: String) : Event
    data object Finishing : Event
    /** [insert] is [text] spaced after what was before the cursor. */
    data class Final(val text: String, val insert: String, val totalMs: Double) : Event
    data object Cancelled : Event
    data class Error(val message: String) : Event

    companion object {
        fun parse(json: String): Event {
            val o = JSONObject(json)
            return when (val type = o.getString("type")) {
                "listening" -> Listening(o.getBoolean("hands_free"))
                "words" -> Words(o.getString("committed"), o.getString("preview"))
                "finishing" -> Finishing
                "final" -> Final(o.getString("text"), o.getString("insert"), o.getDouble("total_ms"))
                "cancelled" -> Cancelled
                "error" -> Error(o.getString("message"))
                else -> Error("unknown event $type")
            }
        }
    }
}
