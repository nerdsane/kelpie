fn main() {
    // Tell Cargo about the madsim cfg so Rust's cfg linter doesn't warn
    // The madsim library sets this cfg when the feature is enabled
    println!("cargo:rustc-check-cfg=cfg(madsim)");
}
