{
  description = "Moltis - Personal AI gateway inspired by OpenClaw";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-compat.url = "github:edolstra/flake-compat";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "moltis";
          version = "0.1.0";
          src = ./.;
          cargoBuildFlags = [ "-p" "moltis" ];
          cargoLock.lockFile = ./Cargo.lock;

          RUSTC = "${rustToolchain}/bin/rustc";
          CARGO = "${rustToolchain}/bin/cargo";

          nativeBuildInputs = with pkgs; [
            perl
            pkg-config
            llvmPackages.clang
            cmake
          ];

          buildInputs = with pkgs; [
            openssl
            pkgs.llvmPackages.libclang.lib
          ];
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

          doCheck = false; # Disable tests due to permission issues in Nix sandbox

          meta = with pkgs.lib; {
            description = "Personal AI gateway inspired by OpenClaw";
            homepage = "https://www.moltis.org/";
            license = licenses.mit;
            mainProgram = "moltis";
          };
        };

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            perl
            pkg-config
            llvmPackages.clang
            cmake
          ];
          buildInputs = with pkgs; [
            rustToolchain
            rust-analyzer
            clippy
            rustfmt
            openssl
            pkgs.llvmPackages.libclang.lib
          ];
          shellHook = ''
            export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
          '';
        };      }
    );
}
