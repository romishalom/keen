// Bakes the build target triple into `whet` so it downloads the right toolchain archive.
fn main() {
    println!("cargo:rustc-env=WHET_TARGET={}", std::env::var("TARGET").unwrap_or_default());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=WHET_DEFAULT_DIST_SERVER");
}
