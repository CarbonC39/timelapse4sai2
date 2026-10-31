{ pkgs ? import <nixpkgs> { } }:
let
  runtimeLibraries = with pkgs; [
    wayland
    libxkbcommon
    libGL
    fontconfig
    (pkgs.libx11 or pkgs.xorg.libX11)
    (pkgs.libxcursor or pkgs.xorg.libXcursor)
    (pkgs.libxi or pkgs.xorg.libXi)
    (pkgs.libxrandr or pkgs.xorg.libXrandr)
    (pkgs.libxcb or pkgs.xorg.libxcb)
  ];
in
pkgs.mkShell {
  packages = with pkgs; [ cargo rustc rustfmt pkg-config ffmpeg ];
  buildInputs = runtimeLibraries;

  shellHook = ''
    # winit loads these libraries at runtime, outside the linker's search path.
    export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath runtimeLibraries}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    if [ -d /run/opengl-driver/lib ]; then
      export LD_LIBRARY_PATH="/run/opengl-driver/lib:$LD_LIBRARY_PATH"
    fi

    # Remote development sessions can inherit a Wayland display that is absent.
    # Preserve native Wayland and inherited socket FDs whenever available.
    if [ -n "''${DISPLAY:-}" ] && [ -n "''${WAYLAND_DISPLAY:-}" ] && [ -z "''${WAYLAND_SOCKET:-}" ]; then
      case "$WAYLAND_DISPLAY" in
        /*) timelapse_wayland_socket="$WAYLAND_DISPLAY" ;;
        *) timelapse_wayland_socket="''${XDG_RUNTIME_DIR:-}/$WAYLAND_DISPLAY" ;;
      esac
      if [ ! -S "$timelapse_wayland_socket" ]; then
        unset WAYLAND_DISPLAY
        echo "Wayland socket unavailable; using X11 ($DISPLAY)."
      fi
      unset timelapse_wayland_socket
    fi
  '';
}
