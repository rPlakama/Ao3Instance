//! Watch the migration directory.
//!
//! `sqlx::migrate!` embeds migrations at compile time but does not ask cargo to
//! watch the directory, so a new/edited migration would not trigger a rebuild
//! and the binary would keep running the old schema. (Same lesson as MUSIKA's
//! `build.rs`.)

fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
