#!/usr/bin/env bash
# Development only: puts the models from a folder on this machine into the debug app's files
# folder, until the first run downloads them: adb pushes them to a temporary folder, and the
# app's own user (run-as, debug builds) copies them in.
#
#   android/native/push-models.sh <adb serial> <folder>
#
# The folder holds speech/ (encoder.tflite, encoder.json, decoder.onnx, joint.onnx,
# config.json, vocab.txt, mel_filters.f32), silero_vad.onnx and the cleanup model's .gguf; a
# test-audio.wav there becomes the microphone of a debug build (Mic.kt).
set -euo pipefail
export MSYS_NO_PATHCONV=1
SERIAL=$1
SRC=$2
APP=dev.utter.app
STAGE=/data/local/tmp/utter-models

adb -s "$SERIAL" shell rm -rf "$STAGE"
adb -s "$SERIAL" shell mkdir -p "$STAGE/models/speech"
for f in encoder.tflite encoder.json decoder.onnx joint.onnx config.json vocab.txt mel_filters.f32; do
  adb -s "$SERIAL" push "$SRC/speech/$f" "$STAGE/models/speech/" | tail -1
done
adb -s "$SERIAL" push "$SRC/silero_vad.onnx" "$SRC"/*.gguf "$STAGE/models/" | tail -1
if [ -f "$SRC/test-audio.wav" ]; then adb -s "$SERIAL" push "$SRC/test-audio.wav" "$STAGE/" | tail -1; fi
adb -s "$SERIAL" shell "chmod -R a+rX $STAGE && run-as $APP cp -r $STAGE/. files/ && rm -r $STAGE"
adb -s "$SERIAL" shell run-as "$APP" ls -lR files
