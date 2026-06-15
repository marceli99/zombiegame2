//! Desktop entry point.  The whole game lives in the library crate (so the
//! same code can be built as an Android `cdylib` via `#[bevy_main]`); this
//! binary is a thin shim that just runs it.
fn main() {
    zombiegame2::run();
}
