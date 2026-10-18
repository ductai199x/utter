#!/bin/sh
# The .deb removed: its udev rule is gone, and the devices go back to what they were.
set -e
if command -v udevadm >/dev/null 2>&1; then
  udevadm control --reload-rules || true
  udevadm trigger --action=change --subsystem-match=input --subsystem-match=misc || true
fi
exit 0
