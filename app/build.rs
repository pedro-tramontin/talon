use std::path::Path;

fn main() {
    // The app crate's tauri::generate_context!() proc macro reads
    // `../ui/dist/index.html` at expansion time and panics if it's
    // missing. The CI workflow (`.github/workflows/ci.yml`) handles
    // building the UI in a separate `ui` job, which uploads the
    // result as a workflow artifact; the `rust` job downloads that
    // artifact before any cargo command runs. For local development,
    // run `pnpm --dir ui build` once before `cargo check / clippy /
    // test`. The check below produces a clear error message if a
    // developer forgets that step.
    //
    // Set TALON_SKIP_UI_BUILD=1 to skip this check entirely (e.g.
    // when working on the Rust lib only with no UI).
    if std::env::var("TALON_SKIP_UI_BUILD").is_ok() {
        println!("cargo:warning=TALON_SKIP_UI_BUILD is set; skipping ui/dist check");
    } else {
        ensure_ui_dist();
    }

    tauri_build::build()
}

fn ensure_ui_dist() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is set by cargo for build scripts");
    let ui_dist = Path::new(&manifest_dir).join("..").join("ui").join("dist");
    let ui_index = ui_dist.join("index.html");

    // Tell cargo to rerun this build script when the UI source or
    // tauri.conf.json changes. This ensures the proc-macro reads a
    // fresh dist if the UI was rebuilt.
    let ui_dir = Path::new(&manifest_dir).join("..").join("ui");
    if ui_dir.exists() {
        println!("cargo:rerun-if-changed={}", ui_dir.display());
    }
    println!(
        "cargo:rerun-if-changed={}",
        Path::new(&manifest_dir).join("tauri.conf.json").display()
    );

    if !ui_dist.exists() || !ui_index.exists() {
        panic!(
            "ui/dist is missing or incomplete ({} not found).\n\
             \n\
             For CI: the `ui` job builds the dist and uploads it as a\n\
             workflow artifact (see .github/workflows/ci.yml). The\n\
             `rust` job downloads that artifact before running cargo.\n\
             If you're seeing this on CI, the artifact pass failed --\n\
             check the UI job's logs for a failed pnpm build.\n\
             \n\
             For local development: run `pnpm --dir ui build` once before\n\
             `cargo check / clippy / test`, or set TALON_SKIP_UI_BUILD=1\n\
             to skip this check (only safe if you're not modifying the UI).",
            ui_index.display(),
        );
    }
}
