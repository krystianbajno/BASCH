fn main() {
    // If you’re using static_vcruntime for Windows, keep it:
    static_vcruntime::metabuild();

    // Emit some environment variables for your crate to read at runtime if desired
    println!("cargo:rustc-env=CARGO_PKG_METADATA_PRECOMPILED_MODE=bind");
    println!("cargo:rustc-env=CARGO_PKG_METADATA_PRECOMPILED_ADDRESS=127.0.0.1:8080");

    // If you ALWAYS want to build in full-PTY mode, keep the line below:
    println!("cargo:rustc-env=CARGO_PKG_METADATA_PRECOMPILED_FULLPTY=TRUE");

    // If you want to conditionally set "FULLPTY" or not, you can read an env var here:
    // e.g. read from your local environment:
    let full_pty_str = std::env::var("CARGO_PKG_METADATA_PRECOMPILED_FULLPTY")
        .unwrap_or_else(|_| "FALSE".to_string());

    // If the var is "TRUE", enable "full_pty" feature; otherwise "partial_pty".
    if full_pty_str.eq_ignore_ascii_case("TRUE") {
        println!("cargo:rustc-cfg=feature=\"full_pty\"");
    } else {
        println!("cargo:rustc-cfg=feature=\"partial_pty\"");
    }
}
