// SPDX-License-Identifier: Apache-2.0
fn main() {
    println!("cargo:rerun-if-changed=linker.ld");
    println!("cargo:rerun-if-env-changed=RUSTIC_BUILD_ID");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none")
        && std::env::var_os("CARGO_FEATURE_BOOT_IMAGE").is_some()
    {
        let root = std::env::var("CARGO_MANIFEST_DIR").expect("Cargo supplies manifest path");
        println!("cargo:rustc-link-arg-bin=rustic-os=-T{root}/linker.ld");
        println!("cargo:rustc-link-arg-bin=rustic-os=--build-id=none");
        println!("cargo:rustc-link-arg-bin=rustic-os=--no-pie");
    }
}
