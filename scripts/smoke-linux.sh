#!/usr/bin/env bash
# Validate the files users download, without requiring FUSE in CI.
set -euo pipefail
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
smoke_gui() {
  local status=0
  env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET LIBGL_ALWAYS_SOFTWARE=1 \
    xvfb-run -a timeout 10 "$@" > "$work/gui.log" 2>&1 || status=$?
  cat "$work/gui.log"
  if [[ "$status" != 124 ]]; then
    echo "GUI exited before the smoke-test timeout (status $status)" >&2
    exit 1
  fi
}
for archive in timelapse4sai2-*-linux-x64-*.tar.gz; do
  tar -xzf "$archive" -C "$work"
done
for binary in "$work"/*/timelapse4sai2; do
  if [[ -f "$binary" ]]; then "$binary" --help; fi
done
for binary in "$work"/*/timelapse4sai2-gui; do
  if [[ -f "$binary" ]]; then smoke_gui "$binary"; fi
done
for image in timelapse4sai2-*-linux-x64-gui.AppImage; do
  if [[ ! -f "$image" ]]; then continue; fi
  image="$PWD/$image"
  python3 scripts/extract-appimage.py "$image" "$work/squashfs-root" > /dev/null
  python3 scripts/check-linux-abi.py "$work/squashfs-root"
  smoke_gui "$work/squashfs-root/AppRun"
done
