{
  description = "A minimal Rust project that fails to build without nightly";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      system = "x86_64-linux";
      overlays = [ (import rust-overlay) ];
      pkgs = import nixpkgs {
        inherit system overlays;
      };
      rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
    in
    {
      packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
        pname = "minimal-rust-app";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        
        RUSTC = "${rustToolchain}/bin/rustc";
        CARGO = "${rustToolchain}/bin/cargo";
      };

      devShells.${system}.default = pkgs.mkShell {
        buildInputs = with pkgs; [
          rustToolchain.rustc
          rustToolchain.cargo
        ];
      };
    };
}
