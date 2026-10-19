// Link the system whisper.cpp (and the ggml it is built on) and generate
// bindings from its headers. GPU support comes from whichever ggml backend
// package is installed (ggml-cuda, ggml-vulkan, ...), loaded at runtime.

fn main() {
    let whisper = pkg_config::Config::new()
        .atleast_version("1.7.6")
        .probe("whisper")
        .expect("whisper.cpp not found via pkg-config (install whisper-cpp)");

    println!("cargo:rerun-if-changed=src/whisper.h");
    let bindings = bindgen::Builder::default()
        .header("src/whisper.h")
        .clang_args(
            whisper
                .include_paths
                .iter()
                .map(|p| format!("-I{}", p.display())),
        )
        .allowlist_function("whisper_.*")
        .allowlist_function("ggml_backend_load_all")
        .allowlist_type("whisper_.*")
        .generate()
        .expect("failed to generate whisper.cpp bindings");

    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out.join("bindings.rs"))
        .expect("failed to write bindings");
}
