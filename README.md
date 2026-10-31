# timelapse4sai2

Record PaintTool SAI 2 canvases as MP4 timelapses, similar to Procreate's
time-lapse feature. The project includes a Rust core, a CLI, and a desktop GUI,
so you can use it as either a command-line tool or a GUI application.

**Supported platforms: Windows x64 and Linux x64.** Both the CLI and desktop GUI
run natively on both operating systems. On Linux, SAI2 itself runs under Wine,
Bottles, or Flatpak.

## SAI2 version support

This project reads SAI2's in-memory canvas data instead of capturing the screen,
so it is tied to specific executable layouts. The following 64-bit builds are
known and tested:

| Build | MD5 | Session RVA |
| --- | --- | --- |
| Preview 2026-07-02b | `6f3f351303cf3896f4c2977925c994c2` | `0x32eb20` |
| Major Renovated 2026-10-19 | `44aee0ba61ea9f215b1b4dcb4d6d23e0` | `0x47a380` |

Unknown **Renovated** builds can be recognized by signature scanning, but they
must have a canvas open before runtime layout validation can complete. SAI
updates may change memory layouts, so automatic matching is not guaranteed.
Other unknown builds are rejected.

If you are using a version older than Preview 2026-07-02b, consider upgrading.

## Requirements

- [PaintTool SAI Ver.2 64-bit](https://www.systemax.jp/en/sai/devdept.html),
  running natively on Windows or under Wine on Linux.
- [FFmpeg](https://ffmpeg.org/download.html) with `libx264` or `h264_nvenc` in
  `PATH` for video export.
- Linux process access: the tool reads SAI memory through `process_vm_readv`.
  `EPERM`/`EACCES` usually means `ptrace_scope`, container permissions, or a
  different user.
- Linux input watching: read access to `/dev/input/event*`. Use
  `--no-input-watch` to record on a timer instead.
- GUI: an X11 or Wayland session with a working OpenGL driver.

## Usage

### Windows portable release

Download the Windows x64 ZIP ending in `-gui.zip` from the releases page and
extract it to a folder. Open `timelapse4sai2-gui.exe` for the desktop app.
The separate `-cli.zip` contains `timelapse4sai2.exe` for terminal use.
Linux releases also provide separate `-gui.tar.gz` and `-cli.tar.gz` archives.
The Windows Visual C++ runtime is linked statically, so
there is no separate runtime installer or DLL bundle to install. FFmpeg must
still be available in `PATH` for video export.

### Build and CLI usage

Build the CLI:

```sh
cargo build --release --locked
```

With SAI2 running:

```sh
# Show the detected build and module base.
./target/release/timelapse4sai2 detect

# List open canvases. The index is only valid for the current canvas list.
./target/release/timelapse4sai2 canvases

# Export one canvas to PNG.
./target/release/timelapse4sai2 dump 0 test.png

# Record canvas 0 into an incremental session.
./target/release/timelapse4sai2 record 0 recordings/my-session
```

Use `--pid 1234` before the subcommand when multiple SAI processes are running.
Stop a recording with Ctrl-C or SIGTERM; the recorder drains pending captures
before exiting.

Inspect, verify, or export a finished session without SAI running:

```sh
./target/release/timelapse4sai2 info recordings/my-session
./target/release/timelapse4sai2 verify recordings/my-session
./target/release/timelapse4sai2 export recordings/my-session artwork.mp4

# Larger output, faster playback, final-artwork preview, and end hold.
./target/release/timelapse4sai2 export recordings/my-session artwork-high.mp4 \
  --size 1080 --pace 60 --preview-end 3 --hold-end 3
```

Useful export options:

```text
--fps 30                 Output FPS
--pace 15                Playback pace in visual captures per second
--encoder auto|nvenc|x264
--quality draft|balanced|high|archival
--size auto|720|1080|1440|2160|50%
--canvas-mode max|initial
--skip-provisional       Export stable captures only
```

`--size` sets the longest edge in pixels, or a percentage of the logical canvas.
See `./target/release/timelapse4sai2 --help` for all options.

### Desktop GUI

```sh
cargo run --release --features gui --bin timelapse4sai2-gui
```

The GUI lets you select a live canvas, record to a session directory, preview
the artwork, and export video. Configure the export parameters first; pressing
**Export video** opens the save dialog and then runs FFmpeg.

## Build and development

```sh
# Include the GUI and run all tests.
cargo test --all-features --locked

# Build the GUI release binary.
cargo build --release --all-features --locked
```

The CLI-only release build is `cargo build --release --locked`. CI runs the
all-features test suite on Linux and Windows and a release build on Windows.
The repository's `.cargo/config.toml` enables static CRT linkage for Windows
x64 MSVC builds, including local builds and release workflows.

### NixOS

```sh
nix-shell
cargo run --release --features gui --bin timelapse4sai2-gui
```

`shell.nix` provides Rust, FFmpeg, and the runtime Wayland/X11/OpenGL libraries.
NixOS does not place these libraries in the global dynamic-loader path, so run
the GUI from inside `nix-shell`; launching the binary from a normal shell fails
with errors such as `NoWaylandLib`. A standalone packaged binary would need a Nix
derivation or wrapper that supplies those runtime dependencies. The shell hook
falls back to X11 when an inherited Wayland socket is unavailable.

## Recording notes

Sessions use incremental tile blobs rather than full frames. Only changed tiles
are written, and export replays them deterministically. Capture is not an atomic
snapshot while SAI is actively painting, so prefer idle moments for validation.
The storage format and recovery guarantees are documented in
[docs/recording-format.md](docs/recording-format.md).

## Acknowledgements

The following projects and documentation informed the implementation and design:

- [cromachina/art-timelapse](https://github.com/cromachina/art-timelapse) —
  reference for SAI2 memory layout, offsets, and the reverse-engineering approach.
- [Krita documentation](https://docs.krita.org/) — engineering reference for
  timelapse recording, replay, and session/storage design.

## License

This project is licensed under the
[GNU General Public License v3.0 only](LICENSE) (GPL-3.0-only).
