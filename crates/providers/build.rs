fn main() {
    // llama-cpp-sys-2's CMake build auto-detects NCCL headers and compiles
    // with GGML_USE_NCCL, but its build.rs never emits cargo:rustc-link-lib=nccl.
    // Bridge that gap here so the final link step can find the NCCL symbols.
    //
    // Only emit the link directive when:
    //  - building for Linux (NCCL is Linux-only)
    //  - the local-llm-cuda feature is enabled
    //  - nccl.h is actually present (CMake only enables NCCL when it finds the header)
    #[cfg(target_os = "linux")]
    if cfg!(feature = "local-llm-cuda") {
        // Check the same paths CMake's FindNCCL.cmake searches
        let nccl_found = ["/usr/include", "/usr/local/include"]
            .iter()
            .any(|dir| std::path::Path::new(dir).join("nccl.h").exists());

        if nccl_found {
            println!("cargo:rustc-link-lib=nccl");
        }
    }
}
