# This file provides a Nix development environment for building Rust projects
# that depend on llama-cpp-sys-2, which requires C++ build tools and libraries.
# 
# On NixOS, these dependencies aren't available by default, so we need to
# explicitly declare them here to ensure successful compilation.
#
# This configuration includes CUDA support for GPU acceleration.
#
# Usage:
# 1. Enter the Nix development environment
#    nix-shell
# 2. Clean and rebuild the project
#    cargo clean
#    cargo build --release

{ pkgs ? import <nixpkgs> { config.allowUnfree = true; } }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    rustc
    cargo
    rustfmt
    clippy
    cmake
    pkg-config
    clang
    stdenv.cc.cc.lib
    cudatoolkit
    linuxPackages.nvidia_x11
  ];
  
  LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
  LD_LIBRARY_PATH = "${pkgs.stdenv.cc.cc.lib}/lib:${pkgs.cudatoolkit}/lib";
  CUDA_PATH = "${pkgs.cudatoolkit}";
  
  shellHook = ''
    export CC=clang
    export CXX=clang++
  '';
}
