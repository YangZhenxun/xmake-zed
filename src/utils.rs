use std::path::PathBuf;

pub fn get_assets_script_path(script_name: String) -> Option<PathBuf> {
    let exe_path = std::env::current_exe().ok()?;
    let cur = exe_path.parent()?;
    let script_path = cur.join("assets").join(script_name);
    if script_path.exists() {
        Some(script_path)
    } else {
        None
    }
}
