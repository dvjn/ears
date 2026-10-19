// Link the system llama.cpp (and the ggml it is built on) and generate
// bindings from its headers. GPU support comes from whichever ggml backend
// package is installed (ggml-cuda, ggml-vulkan, ...), loaded at runtime.

fn main() {
    let llama = pkg_config::Config::new()
        .probe("llama")
        .expect("llama.cpp not found via pkg-config (install llama-cpp)");

    println!("cargo:rerun-if-changed=src/llama.h");
    let bindings = bindgen::Builder::default()
        .header("src/llama.h")
        .clang_args(
            llama
                .include_paths
                .iter()
                .map(|p| format!("-I{}", p.display())),
        )
        .allowlist_function("llama_.*")
        .allowlist_function("ggml_backend_load_all")
        .allowlist_type("llama_.*")
        .generate()
        .expect("failed to generate llama.cpp bindings");

    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out.join("bindings.rs"))
        .expect("failed to write bindings");
}
