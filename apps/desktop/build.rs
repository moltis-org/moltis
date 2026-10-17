//! Points the `mockup` feature at the variants file an agent wrote, named by
//! `MOLTIS_MOCKUP_FILE` (an absolute path outside the repository). Without
//! one, `--mockup` shows its built-in demo.

fn main() {
    println!("cargo:rerun-if-env-changed=MOLTIS_MOCKUP_FILE");
    println!("cargo:rustc-check-cfg=cfg(moltis_mockup_scratch)");
    let Some(file) = std::env::var_os("MOLTIS_MOCKUP_FILE").filter(|file| !file.is_empty()) else {
        return;
    };
    let Some(file) = file.to_str() else {
        panic!("MOLTIS_MOCKUP_FILE must be valid UTF-8");
    };
    if !std::path::Path::new(file).is_absolute() || file.contains(['\n', '\r']) {
        panic!("MOLTIS_MOCKUP_FILE must be an absolute path on one line: {file}");
    }
    println!("cargo:rerun-if-changed={file}");
    println!("cargo:rustc-cfg=moltis_mockup_scratch");
    println!("cargo:rustc-env=MOLTIS_MOCKUP_SCRATCH={file}");
}
