//! Project-pinned `uniffi-bindgen`, version-locked to the `uniffi` runtime
//! this crate links (the two live in one Cargo dependency, so they cannot
//! skew). Library mode is the only mode we use:
//!
//! ```sh
//! cargo run -p genaryx-ffi --bin uniffi-bindgen -- \
//!     generate --library target/release/libgenaryx_ffi.dylib \
//!     --language swift --out-dir <out>
//! ```

fn main() {
    uniffi::uniffi_bindgen_main()
}
