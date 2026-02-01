fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let libkrun_enabled = std::env::var("CARGO_FEATURE_LIBKRUN").is_ok();

    if libkrun_enabled {
        // libkrun uses pkg-config for linking
        // We use manual FFI bindings instead of krun-sys to avoid bindgen/libclang issues
        if let Ok(lib) = pkg_config::Config::new()
            .atleast_version("1.0")
            .probe("libkrun")
        {
            for path in lib.link_paths {
                println!("cargo:rustc-link-search=native={}", path.display());
            }
        } else {
            // Fall back to standard library paths
            if target_os == "macos" {
                println!("cargo:rustc-link-search=native=/usr/local/lib");
                println!("cargo:rustc-link-search=native=/opt/homebrew/lib");
                // Also check user's local lib
                if let Ok(home) = std::env::var("HOME") {
                    println!("cargo:rustc-link-search=native={}/local/lib", home);
                }
            }
        }
        // Link the libkrun library
        println!("cargo:rustc-link-lib=dylib=krun");
    }
}
