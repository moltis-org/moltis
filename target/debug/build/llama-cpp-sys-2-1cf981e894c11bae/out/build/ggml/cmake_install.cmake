# Install script for directory: /home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml

# Set the install prefix
if(NOT DEFINED CMAKE_INSTALL_PREFIX)
  set(CMAKE_INSTALL_PREFIX "/mnt/data1/time-2026/02-february/27/links/moltis/target/debug/build/llama-cpp-sys-2-1cf981e894c11bae/out")
endif()
string(REGEX REPLACE "/$" "" CMAKE_INSTALL_PREFIX "${CMAKE_INSTALL_PREFIX}")

# Set the install configuration name.
if(NOT DEFINED CMAKE_INSTALL_CONFIG_NAME)
  if(BUILD_TYPE)
    string(REGEX REPLACE "^[^A-Za-z0-9_]+" ""
           CMAKE_INSTALL_CONFIG_NAME "${BUILD_TYPE}")
  else()
    set(CMAKE_INSTALL_CONFIG_NAME "Release")
  endif()
  message(STATUS "Install configuration: \"${CMAKE_INSTALL_CONFIG_NAME}\"")
endif()

# Set the component getting installed.
if(NOT CMAKE_INSTALL_COMPONENT)
  if(COMPONENT)
    message(STATUS "Install component: \"${COMPONENT}\"")
    set(CMAKE_INSTALL_COMPONENT "${COMPONENT}")
  else()
    set(CMAKE_INSTALL_COMPONENT)
  endif()
endif()

# Install shared libraries without execute permission?
if(NOT DEFINED CMAKE_INSTALL_SO_NO_EXE)
  set(CMAKE_INSTALL_SO_NO_EXE "0")
endif()

# Is this installation the result of a crosscompile?
if(NOT DEFINED CMAKE_CROSSCOMPILING)
  set(CMAKE_CROSSCOMPILING "FALSE")
endif()

# Set path to fallback-tool for dependency-resolution.
if(NOT DEFINED CMAKE_OBJDUMP)
  set(CMAKE_OBJDUMP "/nix/store/kbw2j1vag664b3sj3rjwz9v53cqx87sb-gcc-wrapper-15.2.0/bin/objdump")
endif()

if(NOT CMAKE_INSTALL_LOCAL_ONLY)
  # Include the install script for the subdirectory.
  include("/mnt/data1/time-2026/02-february/27/links/moltis/target/debug/build/llama-cpp-sys-2-1cf981e894c11bae/out/build/ggml/src/cmake_install.cmake")
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib64" TYPE STATIC_LIBRARY FILES "/mnt/data1/time-2026/02-february/27/links/moltis/target/debug/build/llama-cpp-sys-2-1cf981e894c11bae/out/build/ggml/src/libggml.a")
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/include" TYPE FILE FILES
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml-cpu.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml-alloc.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml-backend.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml-blas.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml-cann.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml-cpp.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml-cuda.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml-opt.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml-metal.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml-rpc.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml-sycl.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml-vulkan.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml-webgpu.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/ggml-zendnn.h"
    "/home/mdupont/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/llama-cpp-sys-2-0.1.133/llama.cpp/ggml/include/gguf.h"
    )
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib64" TYPE STATIC_LIBRARY FILES "/mnt/data1/time-2026/02-february/27/links/moltis/target/debug/build/llama-cpp-sys-2-1cf981e894c11bae/out/build/ggml/src/libggml-base.a")
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/lib64/cmake/ggml" TYPE FILE FILES
    "/mnt/data1/time-2026/02-february/27/links/moltis/target/debug/build/llama-cpp-sys-2-1cf981e894c11bae/out/build/ggml/ggml-config.cmake"
    "/mnt/data1/time-2026/02-february/27/links/moltis/target/debug/build/llama-cpp-sys-2-1cf981e894c11bae/out/build/ggml/ggml-version.cmake"
    )
endif()

string(REPLACE ";" "\n" CMAKE_INSTALL_MANIFEST_CONTENT
       "${CMAKE_INSTALL_MANIFEST_FILES}")
if(CMAKE_INSTALL_LOCAL_ONLY)
  file(WRITE "/mnt/data1/time-2026/02-february/27/links/moltis/target/debug/build/llama-cpp-sys-2-1cf981e894c11bae/out/build/ggml/install_local_manifest.txt"
     "${CMAKE_INSTALL_MANIFEST_CONTENT}")
endif()
