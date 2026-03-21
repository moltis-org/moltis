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
          doCheck = false; # tests run separately in checks.auth-tests

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
        };

        checks = {
          auth-tests = pkgs.rustPlatform.buildRustPackage {
            pname = "moltis-auth-tests";
            version = "0.1.0";
            src = ./.;
            cargoBuildFlags = [ "-p" "moltis-auth" ];
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

            # Ensure tests are run for this check
            doCheck = true;

            preCheck = ''
              export HOME=$(mktemp -d)
              export XDG_CONFIG_HOME="$HOME/.config"
              export XDG_DATA_HOME="$HOME/.local/share"
              export CARGO_HOME="$HOME/.cargo"
              export MOLTIS_CONFIG_DIR="$XDG_CONFIG_HOME/moltis"
              mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$CARGO_HOME" "$XDG_CONFIG_HOME/moltis"
              touch "$XDG_CONFIG_HOME/moltis/moltis.toml"
              trap 'rm -rf $HOME' EXIT
            '';

            checkPhase = ''
              cargo test -p moltis-auth
            '';
          };
        };
      }
    );
}
