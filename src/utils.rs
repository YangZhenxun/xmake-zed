//! Small helpers shared by the extension.

use std::path::PathBuf;

/// The `targetpath.lua` script is embedded into the extension binary so it is
/// always available regardless of how the extension was installed. It is
/// written to the extension's working directory on first use and the absolute
/// path is returned so it can be passed to `xmake l <script> ...`.
///
/// We embed it (rather than reading `assets/targetpath.lua` from disk) because
/// `std::env::current_exe()` is unreliable inside the Zed WASM runtime, which
/// made the old asset-lookup approach fail silently.
pub fn ensure_target_path_script() -> Option<PathBuf> {
    const SCRIPT: &str = include_str!("../assets/targetpath.lua");

    let dir = std::env::current_dir().ok()?;
    let script_path = dir.join("_xmake_zed_targetpath.lua");

    // (Re)write the script if it is missing or out of date.
    let needs_write = match std::fs::read_to_string(&script_path) {
        Ok(existing) => existing != SCRIPT,
        Err(_) => true,
    };
    if needs_write {
        std::fs::write(&script_path, SCRIPT).ok()?;
    }

    Some(script_path)
}

/// Parse the output of `targetpath.lua`, returning the resolved target path
/// (the line between the `__begin__` and `__end__` markers), or `None` when no
/// binary target was found.
pub fn parse_target_path_output(output: &str) -> Option<String> {
    let mut in_section = false;
    let mut path: Option<String> = None;
    for line in output.lines() {
        let line = line.trim();
        if line == "__begin__" {
            in_section = true;
            continue;
        }
        if line == "__end__" {
            break;
        }
        if in_section && !line.is_empty() {
            path = Some(line.to_string());
        }
    }
    path.filter(|p| !p.is_empty())
}
