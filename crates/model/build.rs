//! llama.cpp's Metal backend uses `@available` checks. When Picket targets an older
//! macOS than those checks name, clang emits calls into its own runtime library, which
//! Rust does not link by default, so link it explicitly from the active toolchain.

use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let Ok(output) = Command::new("xcrun")
        .args(["clang", "--print-resource-dir"])
        .output()
    else {
        println!("cargo:warning=xcrun unavailable; clang runtime not linked");
        return;
    };
    let resources = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if resources.is_empty() {
        println!("cargo:warning=clang resource directory not found");
        return;
    }
    println!("cargo:rustc-link-search=native={resources}/lib/darwin");
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
