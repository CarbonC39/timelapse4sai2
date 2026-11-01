#!/usr/bin/env bash
# Run as root inside the Ubuntu 22.04 CI container.
set -euo pipefail
apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  build-essential ca-certificates curl git pkg-config python3 binutils file squashfs-tools \
  libx11-6 libxcursor1 libxi6 libxrandr2 libxcb1 libxkbcommon0 libxkbcommon-x11-0 libx11-xcb1 \
  libwayland-client0 libwayland-cursor0 libwayland-egl1 libfontconfig1 \
  libgl1-mesa-dri libgl1 libegl1 fonts-dejavu-core xvfb xauth desktop-file-utils
