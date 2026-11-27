// Shared by the ears-stt and ears-llm build scripts: build a vendored ggml
// project (whisper.cpp or llama.cpp, git submodules next to this file) as
// static libraries and link them into the server, so the binary carries its
// own ggml and never depends on the system's.
//
// The CPU code targets x86-64-v3 (AVX2, FMA, F16C, BMI2) rather than the build
// machine; set EARS_GGML_NATIVE=1 to tune a local build for its own CPU. The
// `vulkan` feature adds the Vulkan backend, which needs only the Vulkan loader
// (libvulkan.so.1) at runtime.

use std::env;
use std::path::{Path, PathBuf};

/// Build `src` with CMake and link `lib` plus the ggml libraries under it.
/// Returns the include directories for bindgen.
pub fn build(src: &Path, lib: &str, options: &[(&str, &str)]) -> Vec<PathBuf> {
    assert!(
        src.join("CMakeLists.txt").exists(),
        "{} is empty: run `git submodule update --init`",
        src.display()
    );
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-env-changed=EARS_GGML_NATIVE");

    let vulkan = env::var_os("CARGO_FEATURE_VULKAN").is_some();
    let native = env::var("EARS_GGML_NATIVE").is_ok_and(|v| v == "1");
    let on_off = |b: bool| if b { "ON" } else { "OFF" };

    let mut cmake = cmake::Config::new(src);
    // A debug ggml is unusably slow, even in a debug server build.
    cmake
        .profile("Release")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("GGML_BACKEND_DL", "OFF")
        .define("GGML_OPENMP", "OFF")
        .define("GGML_CCACHE", "OFF")
        .define("GGML_NATIVE", on_off(native))
        .define("GGML_VULKAN", on_off(vulkan));
    if !native && env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86_64") {
        for flag in [
            "GGML_AVX",
            "GGML_AVX2",
            "GGML_FMA",
            "GGML_F16C",
            "GGML_BMI2",
        ] {
            cmake.define(flag, "ON");
        }
    }
    for (key, value) in options {
        cmake.define(key, value);
    }
    let out = cmake.build();

    for dir in ["lib", "lib64"] {
        println!("cargo:rustc-link-search=native={}", out.join(dir).display());
    }
    // Static archives resolve left to right: users before what they use.
    let mut libs = vec![lib, "ggml", "ggml-cpu"];
    if vulkan {
        libs.push("ggml-vulkan");
    }
    libs.push("ggml-base");
    for lib in libs {
        println!("cargo:rustc-link-lib=static={lib}");
    }
    if vulkan {
        println!("cargo:rustc-link-lib=dylib=vulkan");
    }
    let cxx = if env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple") {
        "c++"
    } else {
        "stdc++"
    };
    println!("cargo:rustc-link-lib=dylib={cxx}");

    vec![src.join("include"), src.join("ggml/include")]
}
