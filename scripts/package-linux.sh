#!/usr/bin/env bash
# Usage: package-linux.sh TAG [gui|cli|all]
set -euo pipefail
tag="${1:?release tag required}"
edition="${2:-all}"
case "$edition" in gui|cli|all) ;; *) exit 2 ;; esac
for item in gui cli; do
  if [[ "$edition" != all && "$edition" != "$item" ]]; then continue; fi
  name="timelapse4sai2-$tag-linux-x64-$item"
  binary=timelapse4sai2
  if [[ "$item" == gui ]]; then binary=timelapse4sai2-gui; fi
  mkdir -p "dist/$name"
  install -m 755 "target/release/$binary" "dist/$name/"
  cp README.md LICENSE shell.nix "dist/$name/"
  python3 scripts/check-linux-abi.py "dist/$name/$binary"
  tar -C dist -czf "$name.tar.gz" "$name"
done
if [[ "$edition" == cli ]]; then exit; fi
appdir="$PWD/dist/timelapse4sai2.AppDir"
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/lib" "$appdir/usr/share/doc/timelapse4sai2"
install -m 755 target/release/timelapse4sai2{,-gui} "$appdir/usr/bin/"
cp README.md LICENSE "$appdir/usr/share/doc/timelapse4sai2/"
cp resources/linux/timelapse4sai2.{desktop,svg} "$appdir/"
desktop-file-validate "$appdir/timelapse4sai2.desktop"
# winit loads these with dlopen, so they are absent from the executable's NEEDED entries.
# Leave glibc and graphics drivers to the host; bundle the window-system client libraries.
python3 - "$appdir" <<'PY'
import pathlib, re, shutil, subprocess, sys
root = pathlib.Path(sys.argv[1])
cache = subprocess.check_output(['ldconfig', '-p'], text=True)
seeds = ['libX11.so.6', 'libXcursor.so.1', 'libXi.so.6', 'libXrandr.so.2',
         'libxcb.so.1', 'libxkbcommon.so.0', 'libxkbcommon-x11.so.0',
         'libX11-xcb.so.1', 'libwayland-client.so.0',
         'libwayland-cursor.so.0', 'libwayland-egl.so.1', 'libfontconfig.so.1']
queue = list((root / 'usr/bin').iterdir())
for name in seeds:
    match = re.search(r'^\s*' + re.escape(name) + r' .* => (\S+)', cache, re.M)
    if not match:
        raise SystemExit(f'Missing runtime library: {name}')
    queue.append(pathlib.Path(match[1]))
exclude = re.compile(r'^(ld-linux.*|lib(c|m|pthread|dl|rt|resolv|util|anl)\.so\..*)$')
seen = set()
license_dir = root / 'usr/share/doc/bundled-libraries'
license_dir.mkdir(parents=True)
shutil.copytree('/usr/share/common-licenses', license_dir / 'common-licenses')
while queue:
    path = queue.pop()
    if path.name in seen or exclude.match(path.name):
        continue
    seen.add(path.name)
    if path.parent != root / 'usr/bin':
        shutil.copy2(path.resolve(), root / 'usr/lib' / path.name)
        owner = subprocess.check_output(['dpkg-query', '-S', '*/' + path.name], text=True).split(': ')[0]
        package = owner.split(':')[0]
        copyright_file = pathlib.Path('/usr/share/doc') / package / 'copyright'
        if not copyright_file.exists():
            raise SystemExit(f'Missing copyright file for {path}')
        shutil.copy2(copyright_file, license_dir / (package + '.copyright'))
    output = subprocess.check_output(['ldd', str(path)], text=True)
    if 'not found' in output:
        raise SystemExit(output)
    queue.extend(pathlib.Path(p) for p in re.findall(r'=> (/\S+)', output))
PY
cat > "$appdir/AppRun" <<'RUN'
#!/bin/sh
appdir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export LD_LIBRARY_PATH="$appdir/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$appdir/usr/bin/timelapse4sai2-gui" "$@"
RUN
chmod +x "$appdir/AppRun"
python3 scripts/check-linux-abi.py "$appdir"
# Pin both the packaging tool and runtime, including their download checksums.
tools=$(mktemp -d)
trap 'rm -rf "$tools"' EXIT
curl -fL --retry 3 https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage -o "$tools/appimagetool"
curl -fL --retry 3 https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64 -o "$tools/runtime"
echo "ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0  $tools/appimagetool" | sha256sum -c -
echo "2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d  $tools/runtime" | sha256sum -c -
chmod +x "$tools/appimagetool"
python3 scripts/extract-appimage.py "$tools/appimagetool" "$tools/squashfs-root" > /dev/null
ARCH=x86_64 "$tools/squashfs-root/AppRun" --runtime-file "$tools/runtime" --no-appstream "$appdir" \
  "$PWD/timelapse4sai2-$tag-linux-x64-gui.AppImage"
