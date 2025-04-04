fn main() {
    static_vcruntime::metabuild();

    println!("cargo:rustc-env=CARGO_PKG_METADATA_PRECOMPILED_MODE=bind");
    println!("cargo:rustc-env=CARGO_PKG_METADATA_PRECOMPILED_ADDRESS=127.0.0.1:8080");
}
