//! Emit the C header (`yeti_sdk_go.h`) for the cgo binding from the crate's
//! `#[no_mangle] extern "C"` surface, via cbindgen. The Go package's
//! `build.sh` runs this and stages the header next to the staticlib.
//!
//!   cargo run --bin yeti-sdk-go-header -- <out-header-path>

#![allow(clippy::print_stderr)]

use std::path::PathBuf;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "yeti_sdk_go.h".to_owned());
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let config = cbindgen::Config {
        language: cbindgen::Language::C,
        cpp_compat: true,
        pragma_once: true,
        // The Go cgo preamble includes this header; a clean, dependency-free C
        // header (only <stdint.h>/<stdbool.h>/<stddef.h>) is what we want.
        sys_includes: vec![
            "stdint.h".to_owned(),
            "stdbool.h".to_owned(),
            "stddef.h".to_owned(),
        ],
        no_includes: true,
        ..Default::default()
    };

    cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config)
        .generate()
        .expect("generate C header for the Yeti Go binding")
        .write_to_file(&out);

    eprintln!("wrote {out}");
}
