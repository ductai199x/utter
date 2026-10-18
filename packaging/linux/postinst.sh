#!/bin/sh
# The .deb's udev rule (70-utter.rules) applied to the devices there now, so the dictation key
# and the paste keys work from the first run, without logging out.
set -e
modprobe uinput 2>/dev/null || true
if command -v udevadm >/dev/null 2>&1; then
  udevadm control --reload-rules || true
  udevadm trigger --action=change --subsystem-match=input --subsystem-match=misc || true
fi
exit 0
