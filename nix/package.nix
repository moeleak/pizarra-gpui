{
  lib,
  rustPlatform,
  pkg-config,
  fontconfig,
  freetype,
  libxkbcommon,
  libxcb,
  libx11,
  wayland,
  vulkan-loader,
}:

let
  cargoToml = lib.importTOML ../Cargo.toml;

  # GPUI dlopen()s these at runtime instead of linking them, so they must be
  # added to the binary's RUNPATH (and to LD_LIBRARY_PATH in the dev shell).
  runtimeLibs = [
    vulkan-loader
    wayland
    fontconfig
    libxkbcommon
    libxcb
    libx11
  ];
in
rustPlatform.buildRustPackage {
  pname = cargoToml.package.name;
  inherit (cargoToml.package) version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../res
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [ pkg-config ];

  buildInputs = [
    fontconfig
    freetype
    libxkbcommon
    libxcb
    libx11
    wayland
    vulkan-loader
  ];

  postInstall = ''
    install -Dm644 res/pizarra-gpui.desktop $out/share/applications/pizarra-gpui.desktop
    install -Dm644 res/pizarra.svg $out/share/icons/hicolor/scalable/apps/pizarra-gpui.svg
    install -Dm644 res/pizarra128.png $out/share/icons/hicolor/128x128/apps/pizarra-gpui.png
  '';

  postFixup = ''
    patchelf --add-rpath ${lib.makeLibraryPath runtimeLibs} $out/bin/pizarra-gpui
  '';

  passthru = { inherit runtimeLibs; };

  meta = {
    description = "Free-hand vector drawing application with an infinite canvas (GPUI frontend)";
    homepage = "https://pizarra.categulario.xyz";
    license = lib.licenses.gpl3Plus;
    mainProgram = "pizarra-gpui";
    platforms = lib.platforms.linux;
  };
}
