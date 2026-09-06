//! Entry point.  The whole game lives in the library crate; this binary is a
//! thin shim that `build-web.sh` compiles to wasm (wasm-bindgen calls `main`
//! on load) and that also runs natively for single player and the tests.
fn main() {
    zombiegame2::run();
}
