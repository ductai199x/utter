package dev.utter.app

import android.content.Context
import java.io.File

/**
 * Where the models live, in the app's files folder:
 * - `models/speech/`: encoder.tflite and encoder.json (utter-speech export-litert), decoder.onnx,
 *   joint.onnx, config.json, vocab.txt, mel_filters.f32 (utter-speech export);
 * - `models/silero_vad.onnx`;
 * - `models/<name>.gguf`: the cleanup model (none: the transcript goes in as heard).
 */
class Models(context: Context) {
    val dir = File(context.filesDir, "models")
    val speech = File(dir, "speech")
    val vad = File(dir, "silero_vad.onnx")
    val cleanup: File? get() = dir.listFiles { f -> f.name.endsWith(".gguf") }?.minByOrNull { it.name }

    private val speechFiles =
        listOf("encoder.tflite", "encoder.json", "decoder.onnx", "joint.onnx", "config.json", "vocab.txt", "mel_filters.f32")

    /** The files dictation cannot start without that are not there. */
    fun missing(): List<String> =
        speechFiles.filter { !File(speech, it).isFile }.map { "speech/$it" } + listOfNotNull("silero_vad.onnx".takeIf { !vad.isFile })
}
