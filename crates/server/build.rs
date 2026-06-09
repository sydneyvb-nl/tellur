//! Ensure the embedded dashboard asset directory exists so the crate always
//! compiles, even when the SPA hasn't been built (`pnpm --dir ui build`). When
//! `ui/dist` is empty the hub serves a small placeholder at `/app`.

use std::path::Path;

fn main() {
    let dist = Path::new("ui/dist");
    if !dist.exists() {
        let _ = std::fs::create_dir_all(dist);
    }
    println!("cargo:rerun-if-changed=ui/dist");
}
