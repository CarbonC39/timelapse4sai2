{
  lib,
  rustPlatform,
  makeWrapper,
  pkg-config,
  ffmpeg,
  xdg-utils,
  runtimeLibraries,
  withGui ? true,
}:
let
  manifest = builtins.fromTOML (builtins.readFile ../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = manifest.package.name + lib.optionalString (!withGui) "-cli";
  inherit (manifest.package) version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../build.rs
      ../src
      ../resources
      ../README.md
      ../LICENSE
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;
  buildFeatures = lib.optional withGui "gui";
  nativeBuildInputs = [ makeWrapper pkg-config ];

  postInstall = ''
    wrapProgram "$out/bin/timelapse4sai2" \
      --prefix PATH : ${lib.makeBinPath [ ffmpeg ]}
    install -Dm644 LICENSE "$out/share/licenses/timelapse4sai2/LICENSE"
  '' + lib.optionalString withGui ''
    wrapProgram "$out/bin/timelapse4sai2-gui" \
      --prefix PATH : ${lib.makeBinPath [ ffmpeg xdg-utils ]} \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibraries} \
      --prefix LD_LIBRARY_PATH : /run/opengl-driver/lib
    install -Dm644 resources/linux/timelapse4sai2.desktop \
      "$out/share/applications/timelapse4sai2.desktop"
    install -Dm644 resources/linux/timelapse4sai2.svg \
      "$out/share/icons/hicolor/scalable/apps/timelapse4sai2.svg"
  '';

  meta = {
    description = "Record PaintTool SAI 2 canvas timelapses";
    homepage = "https://github.com/CarbonC39/timelapse4sai2";
    license = lib.licenses.gpl3Only;
    platforms = [ "x86_64-linux" ];
    mainProgram = if withGui then "timelapse4sai2-gui" else "timelapse4sai2";
  };
}
