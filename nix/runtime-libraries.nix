pkgs:
with pkgs;
[
  wayland
  libxkbcommon
  libGL
  fontconfig
  (pkgs.libx11 or pkgs.xorg.libX11)
  (pkgs.libxcursor or pkgs.xorg.libXcursor)
  (pkgs.libxi or pkgs.xorg.libXi)
  (pkgs.libxrandr or pkgs.xorg.libXrandr)
  (pkgs.libxcb or pkgs.xorg.libxcb)
]
