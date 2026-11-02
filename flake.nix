{
  description = "Pizarra — a free-hand vector drawing application with an infinite canvas, rewritten on GPUI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      inherit (nixpkgs) lib;

      # GPUI's Linux backend (Vulkan via blade, Wayland + X11) is what this
      # flake builds and tests. macOS would need the Metal toolchain from
      # Xcode, which a pure Nix build cannot provide.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      forAllSystems = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: {
        pizarra-gpui = pkgs.callPackage ./nix/package.nix { };
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.pizarra-gpui;
      });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.default;
          meta.description = "Run Pizarra";
        };
      });

      devShells = forAllSystems (
        pkgs:
        let
          pizarra = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ pizarra ];

            packages = with pkgs; [
              cargo
              rustc
              clippy
              rustfmt
              rust-analyzer
            ];

            # Vulkan, Wayland and X11 libraries are dlopen()ed at runtime by
            # GPUI, so `cargo run` needs to find them.
            LD_LIBRARY_PATH = lib.makeLibraryPath pizarra.runtimeLibs;
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          };
        }
      );

      checks = forAllSystems (pkgs: {
        package = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt-rfc-style);
    };
}
