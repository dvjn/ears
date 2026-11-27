// Build the vendored llama.cpp into ears-llm and generate bindings from its
// header. See vendor/ggml.rs.

#[path = "../vendor/ggml.rs"]
mod ggml;

use std::path::{Path, PathBuf};

fn main() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../vendor/llama.cpp");
    let includes = ggml::build(
        &src,
        "llama",
        &[
            ("LLAMA_BUILD_COMMON", "OFF"),
            ("LLAMA_BUILD_TESTS", "OFF"),
            ("LLAMA_BUILD_TOOLS", "OFF"),
            ("LLAMA_BUILD_EXAMPLES", "OFF"),
            ("LLAMA_BUILD_SERVER", "OFF"),
            ("LLAMA_BUILD_APP", "OFF"),
            ("LLAMA_OPENSSL", "OFF"),
        ],
    );

    let bindings = bindgen::Builder::default()
        .header(src.join("include/llama.h").to_string_lossy())
        .clang_args(includes.iter().map(|p| format!("-I{}", p.display())))
        .allowlist_function("llama_.*")
        .allowlist_type("llama_.*")
        .generate()
        .expect("failed to generate llama.cpp bindings");

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out.join("bindings.rs"))
        .expect("failed to write bindings");
}
