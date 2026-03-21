
#![no_std]

// `no_mangle` forces codegen, which makes llvm check the contents of the `asm!` macro
#[no_mangle]
unsafe fn asm() {
    core::arch::asm!("clrex");
}
