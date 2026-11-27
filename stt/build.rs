// Build the vendored whisper.cpp into ears-stt and generate bindings from its
// header. See vendor/ggml.rs.

#[path = "../vendor/ggml.rs"]
mod ggml;

use std::path::{Path, PathBuf};

fn main() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../vendor/whisper.cpp");
    let includes = ggml::build(
        &src,
        "whisper",
        &[
            ("WHISPER_BUILD_TESTS", "OFF"),
            ("WHISPER_BUILD_EXAMPLES", "OFF"),
            ("WHISPER_BUILD_SERVER", "OFF"),
        ],
    );

    let bindings = bindgen::Builder::default()
        .header(src.join("include/whisper.h").to_string_lossy())
        .clang_args(includes.iter().map(|p| format!("-I{}", p.display())))
        .allowlist_function("whisper_.*")
        .allowlist_type("whisper_.*")
        .generate()
        .expect("failed to generate whisper.cpp bindings");

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out.join("bindings.rs"))
        .expect("failed to write bindings");
}
