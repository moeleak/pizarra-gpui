{
  lib,
  stdenv,
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
  runtimeLibs = lib.optionals stdenv.hostPlatform.isLinux [
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
    root = ./..;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../res
    ];
  };

  cargoLock = {
    lockFile = ../Cargo.lock;
    # One hash per git revision: the engine and GPUI forks, and the
    # zed-industries forks GPUI pulls in.
    outputHashes = {
      "gpui-0.2.2" = "sha256-ZyRdhu54D9wEwpiwwn3laNO76E4lysoZmPex3j9zqfU=";
      "pizarra-3.0.1" = "sha256-MdwARIP+P31WF4HM6Ze+iK0//aBDP1qAZubJZBNTRbQ=";
      "zed-xim-0.4.0-zed" = "sha256-pRT4Sz1JU9ros47/7pmIW9kosWOGMOItcnNd+VrvnpE=";
      "zed-async-tar-0.5.0-zed" = "sha256-CpDHRg6+6u6GnhAVEtqupOsko/6/HDDDl2orCfa8vCY=";
      "zed-font-kit-0.14.1-zed" = "sha256-rxpumYP0QpHW+4e+J1qo5lEZXfBk1LaL/Y0APkUp9cg=";
      "zed-reqwest-0.12.15-zed" = "sha256-p4SiUrOrbTlk/3bBrzN/mq/t+1Gzy2ot4nso6w6S+F8=";
      "zed-scap-0.0.8-zed" = "sha256-BihiQHlal/eRsktyf0GI3aSWsUCW7WcICMsC2Xvb7kw=";
    };
  };

  nativeBuildInputs = [
    pkg-config
  ]
  ++ lib.optionals stdenv.hostPlatform.isDarwin [ rustPlatform.bindgenHook ];

  buildInputs = runtimeLibs ++ lib.optionals stdenv.hostPlatform.isLinux [ freetype ];

  # The Metal shader compiler only ships with Xcode, so compile the shaders
  # when the app starts instead.
  buildFeatures = lib.optionals stdenv.hostPlatform.isDarwin [ "gpui/runtime_shaders" ];

  postInstall = lib.optionalString stdenv.hostPlatform.isLinux ''
    install -Dm644 res/pizarra-gpui.desktop $out/share/applications/pizarra-gpui.desktop
    install -Dm644 res/pizarra.svg $out/share/icons/hicolor/scalable/apps/pizarra-gpui.svg
    install -Dm644 res/pizarra128.png $out/share/icons/hicolor/128x128/apps/pizarra-gpui.png
  '';

  postFixup = lib.optionalString stdenv.hostPlatform.isLinux ''
    patchelf --add-rpath ${lib.makeLibraryPath runtimeLibs} $out/bin/pizarra-gpui
  '';

  passthru = { inherit runtimeLibs; };

  meta = {
    description = "Free-hand vector drawing application with an infinite canvas (GPUI frontend)";
    homepage = "https://github.com/moeleak/pizarra-gpui";
    license = lib.licenses.gpl3Plus;
    mainProgram = "pizarra-gpui";
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
}
