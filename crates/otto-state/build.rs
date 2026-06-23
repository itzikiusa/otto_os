// Force a rebuild whenever the SQL migration set changes. `sqlx::migrate!()`
// embeds `migrations/` at COMPILE time, but adding a new `.sql` file does not by
// itself touch any tracked Rust source — so an incremental build would otherwise
// reuse the cached crate and ship a stale migration set (the daemon would then
// never apply the newest migration). Tracking the directory closes that gap.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
