{ pkgs, rustToolchain }:
pkgs.rustPlatform.buildRustPackage {
  pname = "moltis";
  version = "0.1.0";
  src = ./.;
  cargoBuildFlags = [ "-p" "moltis" ];
  cargoLock.lockFile = ./Cargo.lock;

  toolchain = rustToolchain;

  nativeBuildInputs = with pkgs; [
    perl
    pkg-config
    llvmPackages.clang
  ];

  buildInputs = with pkgs; [
    openssl
    llvmPackages.libclang
  ];

  meta = with pkgs.lib; {
    description = "Personal AI gateway inspired by OpenClaw";
    homepage = "https://www.moltis.org/";
    license = licenses.mit;
    mainProgram = "moltis";
  };
}
