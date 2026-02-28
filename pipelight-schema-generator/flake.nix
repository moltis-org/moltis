{
  description = "A Rust project with Nix flake to generate schemas for pipelight job data";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    buildbot-nix.url = "github:meta-introspector/buildbot-nix"; # Added buildbot-nix as an input
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, buildbot-nix, ... }@inputs:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      in
      {
        devShells.default = pkgs.mkShell {
          inputsFrom = [ buildbot-nix.devShells.default ]; # Inherit devShells from buildbot-nix
          buildInputs = [
            rustToolchain
          ];
          RUST_SRC_PATH = rustToolchain.rust-src;
        };
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "pipelight-schema-generator";
          version = "0.1.0";
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
        };
      }
    );
}
