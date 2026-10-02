use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use zed_extension_api::{self as zed, LanguageServerId, Result, Worktree, settings::LspSettings};

mod utils;

/// The user-facing debug configuration. Mirrors the schema in
/// `debug_adapter_schemas/xmake.json` and the VS Code xmake extension so that
/// `debug.json` files stay portable between editors.
///
/// Field names are camelCase on the wire (note `stop_at_entry` -> `stopAtEntry`)
/// because the underlying debug adapters (lldb-dap / gdb-dap) use that spelling.
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase", default)]
struct XMakeDebugConfig {
    /// Absolute path to the binary to debug. Resolved by the `xmake` debug
    /// locator after building the target.
    program: Option<String>,
    /// Command-line arguments forwarded to the debugged program.
    args: Option<Vec<String>>,
    /// Working directory for the debugged program.
    cwd: Option<String>,
    /// Extra environment variables for the debugged program.
    #[serde(default)]
    env: HashMap<String, String>,
    /// `"launch"` (default) or `"attach"`.
    request: String,
    /// Whether to stop immediately after launching.
    stop_at_entry: Option<bool>,
    /// Process id to attach to (only meaningful for `request: "attach"`).
    pid: Option<u32>,
    /// Which low-level debugger to drive. Defaults to `"codelldb"` (see the
    /// crate-level note on why CodeLLDB is preferred over `lldb-dap`). Other
    /// accepted values: `"lldb-dap"`, `"gdb-dap"`.
    debugger: Option<String>,
    /// Where the debugged program's stdio is connected. Forwarded to the
    /// underlying adapter's launch config (see `to_adapter_config`):
    ///   - `"internalConsole"` → read-only Debug Console. Reliable;
    ///     breakpoints/stepping/variables all work, but stdin cannot be
    ///     typed into. This is the *forced* default for `lldb-dap` because its
    ///     own "runInTerminal launcher" mechanism times out on Zed.
    ///   - `"integratedTerminal"` → an interactive terminal, so stdin works.
    ///     CodeLLDB uses the standard DAP `runInTerminal` reverse request
    ///     (which Zed supports) and is reliable; `lldb-dap` only reaches this
    ///     through its fragile FIFO launcher. If omitted, the default is
    ///     inferred from the `debugger`: `integratedTerminal` for CodeLLDB,
    ///     `internalConsole` for lldb-dap.
    console: Option<String>,
    /// Display label for the debug scenario.
    label: String,
}

impl Default for XMakeDebugConfig {
    fn default() -> Self {
        Self {
            program: None,
            args: None,
            cwd: None,
            env: HashMap::new(),
            request: "launch".to_string(),
            stop_at_entry: None,
            pid: None,
            debugger: None,
            console: None,
            label: "xmake debug".to_string(),
        }
    }
}

impl XMakeDebugConfig {
    /// Build the JSON configuration that the *underlying* debug adapter
    /// (lldb-dap / gdb-dap) understands. The xmake wrapper fields such as
    /// `debugger`, `label` and `request` are intentionally dropped — they are
    /// not part of the adapter's schema and only add noise.
    ///
    /// `stop_at_entry` is translated to the spelling each adapter expects.
    ///
    /// `console` handling — see README "Debugging" section for the full story:
    ///
    /// lldb-dap's `console: "integratedTerminal"` (and the deprecated
    /// `runInTerminal: true`) triggers lldb-dap's *own* "runInTerminal
    /// launcher" mechanism (a FIFO-based child process, NOT the standard DAP
    /// `runInTerminal` reverse request). That launcher re-execs the lldb-dap
    /// binary with `--comm-file <fifo> --launch-target <program>`, and the
    /// main process waits for the child to report the target PID over the
    /// FIFO. On Zed this frequently times out — "Timed out trying to get
    /// messages from the runInTerminal launcher" — because the launcher child
    /// fails to start the target (paths containing spaces, shell quoting,
    /// FIFO permission issues, etc.).
    ///
    /// We therefore default to `"internalConsole"`: the debug session runs in
    /// the read-only Debug Console, which is reliable. Breakpoints, stepping
    /// and variable inspection all work; only interactive stdin is
    /// unavailable. Users who need stdin should run `xmake run` in Zed's
    /// Terminal panel, or explicitly set `"console": "integratedTerminal"` in
    /// their `.zed/debug.json` and accept the timeout risk.
    fn to_adapter_config(&self) -> serde_json::Value {
        let stop = self.stop_at_entry.unwrap_or(false);
        let debugger = self.debugger.as_deref().unwrap_or("codelldb");
        // CodeLLDB uses the standard DAP `runInTerminal` reverse request (which
        // Zed supports), so interactive stdin works reliably. We therefore
        // default its `console` to `integratedTerminal` so programs that read
        // from stdin just work. lldb-dap only supports interactive stdin
        // through its *own* "runInTerminal launcher" FIFO mechanism, which
        // times out on Zed (see README "Debugging"), so we force its `console`
        // to `internalConsole` unless the user explicitly opts in.
        let console = self.console.clone().unwrap_or_else(|| {
            if debugger == "lldb-dap" {
                "internalConsole".to_string()
            } else {
                "integratedTerminal".to_string()
            }
        });
        let mut cfg = serde_json::json!({
            "program": self.program.clone().unwrap_or_default(),
            "args": self.args.clone().unwrap_or_default(),
            "cwd": self.cwd.clone().unwrap_or_default(),
            "env": self.env,
        });
        let obj = cfg.as_object_mut().expect("json! builds an object");
        match debugger {
            "lldb-dap" => {
                obj.insert("stopOnEntry".to_string(), serde_json::Value::Bool(stop));
                // Forward `console` so users who explicitly opt into
                // `integratedTerminal` (in .zed/debug.json) get their wish.
                // We do NOT set the deprecated `runInTerminal` field: lldb-dap
                // 21.0+ parses both into the same internal `Console` enum
                // (ProtocolRequests.cpp L310-311), so `console` alone
                // suffices and avoids double-triggering on older builds.
                obj.insert("console".to_string(), serde_json::Value::String(console));
            }
            "codelldb" => {
                obj.insert("stopOnEntry".to_string(), serde_json::Value::Bool(stop));
                // CodeLLDB understands `console: integratedTerminal` and uses
                // the standard DAP runInTerminal reverse request — interactive
                // stdin works. `name` is used as the terminal title.
                obj.insert("console".to_string(), serde_json::Value::String(console));
                obj.insert("name".to_string(), serde_json::Value::String(self.label.clone()));
            }
            "gdb-dap" => {
                // gdb-dap does not understand `console`; it always launches via
                // the client's `runInTerminal` reverse request when the client
                // supports it, so we leave it alone here.
                obj.insert(
                    "stopAtBeginningOfMainSubprogram".to_string(),
                    serde_json::Value::Bool(stop),
                );
                obj.insert("stopOnEntry".to_string(), serde_json::Value::Bool(stop));
            }
            _ => {
                obj.insert("stopOnEntry".to_string(), serde_json::Value::Bool(stop));
            }
        }
        cfg
    }
}

struct XMakeExtension {
    cached_binary_path: Option<String>,
    cached_codelldb_path: Option<String>,
}

impl XMakeExtension {
    fn get_linux_variant(&self, worktree: &Worktree) -> Result<String> {
        let settings = LspSettings::for_worktree("xmake-ls", worktree).ok();

        if let Some(settings) = settings {
            if let Some(settings_value) = settings.settings {
                if let Some(variant) = settings_value.get("linuxVariant") {
                    if let Some(variant_str) = variant.as_str() {
                        return Ok(variant_str.to_string());
                    }
                }
            }
        }

        let (_, arch) = zed::current_platform();
        match arch {
            zed::Architecture::Aarch64 => Ok("aarch64-glibc.2.17".to_string()),
            zed::Architecture::X8664 => Ok("x64-glibc.2.17".to_string()),
            zed::Architecture::X86 => {
                Err("32-bit x86 Linux is not supported by xmake_ls".to_string())
            }
        }
    }

    fn get_settings(&self, worktree: &Worktree) -> Result<Option<zed::serde_json::Value>> {
        let settings = LspSettings::for_worktree("xmake-ls", worktree).ok();
        Ok(settings.and_then(|s| s.settings))
    }

    fn find_lldb_dap(
        &self,
        worktree: &zed_extension_api::Worktree,
    ) -> Result<(String, Option<String>), String> {
        let (platform, _) = zed::current_platform();
        match platform {
            zed::Os::Mac => {
                if let Some(xcrun_path) = worktree.which("xcrun") {
                    return Ok((xcrun_path, Some("lldb-dap".into())));
                }
                if let Some(path) = worktree.which("lldb-dap") {
                    return Ok((path, None));
                }
                let homebrew_paths = vec![
                    "/opt/homebrew/bin/lldb-dap".to_string(),
                    "/usr/local/bin/lldb-dap".to_string(),
                ];
                for path in homebrew_paths {
                    if std::path::Path::new(&path).exists() {
                        return Ok((path, None));
                    }
                }
                let xcode_path = "/usr/bin/lldb-dap".to_string();
                if std::path::Path::new(&xcode_path).exists() {
                    return Ok((xcode_path, None));
                }
            }
            zed::Os::Linux => {
                if let Some(path) = worktree.which("lldb-dap") {
                    return Ok((path, None));
                }
                if let Some(path) = worktree.which("lldb-dap-20") {
                    return Ok((path, None));
                }
                let common_paths = vec![
                    "/usr/bin/lldb-dap".to_string(),
                    "/usr/local/bin/lldb-dap".to_string(),
                ];
                for path in common_paths {
                    if std::path::Path::new(&path).exists() {
                        return Ok((path, None));
                    }
                }
            }

            zed::Os::Windows => {
                if let Some(path) = worktree.which("lldb-dap.exe") {
                    return Ok((path, None));
                }
                let program_files = std::env::var("ProgramFiles")
                    .unwrap_or_else(|_| "C:\\Program Files".to_string());
                let program_files_x86 = std::env::var("ProgramFiles(x86)")
                    .unwrap_or_else(|_| "C:\\Program Files (x86)".to_string());

                let common_paths = vec![
                    format!("{}\\LLVM\\bin\\lldb-dap.exe", program_files),
                    format!("{}\\LLVM\\bin\\lldb-dap.exe", program_files_x86),
                    "C:\\msys64\\mingw64\\bin\\lldb-dap.exe".to_string(),
                ];
                for path in common_paths {
                    if std::path::Path::new(&path).exists() {
                        return Ok((path, None));
                    }
                }
            }
        }
        let way_of_installation = match platform {
            zed::Os::Mac => "`brew install llvm` or ensure Xcode 16+ is installed.",
            zed::Os::Linux => {
                "`sudo apt install lldb` (Ubuntu/Debian) or `sudo pacman -S lldb` (Arch)."
            }
            zed::Os::Windows => "Install LLVM from https://llvm.org and add it to PATH.",
        };
        Err(format!(
            "Could not find lldb-dap. Please install it via:\n{}",
            way_of_installation
        ))
    }

    fn find_gdb_dap(
        &self,
        worktree: &zed_extension_api::Worktree,
    ) -> Result<(String, Vec<String>), String> {
        let (platform, _) = zed::current_platform();
        let gdb_path = match platform {
            zed::Os::Windows => worktree.which("gdb.exe").or_else(|| worktree.which("gdb")),
            _ => worktree.which("gdb"),
        };

        match gdb_path {
            Some(path) => Ok((path, vec!["-i".to_string(), "dap".to_string()])),
            None => {
                let way_of_installation = match platform {
                    zed::Os::Mac => "`brew install gdb`",
                    zed::Os::Linux => "`sudo apt install gdb` (Ubuntu/Debian)",
                    zed::Os::Windows => "Install MinGW or MSYS2 and add gdb to PATH.",
                };
                Err(format!(
                    "Could not find gdb. Please install it:\n{}",
                    way_of_installation
                ))
            }
        }
    }

    /// Resolve the CodeLLDB debug adapter binary, downloading it on first use.
    ///
    /// CodeLLDB (`vadimcn/codelldb`) is a *native* DAP adapter bundled inside a
    /// `.vsix` package at `extension/adapter/codelldb`. Unlike `lldb-dap`, it
    /// uses the standard DAP `runInTerminal` reverse request — which Zed
    /// supports — so interactive stdin works reliably inside Zed's terminal.
    /// That is why Zed's built-in Rust/C++ debugging can accept terminal input
    /// while `lldb-dap` cannot.
    ///
    /// The `.vsix` is fetched from the latest GitHub release and extracted via
    /// `download_file(..., Zip)` (a vsix *is* a zip). The adapter binary lives
    /// at `<version_dir>/extension/adapter/codelldb{,.exe}`.
    fn find_codelldb(&mut self, _worktree: &Worktree) -> Result<(String, Vec<String>), String> {
        // Return the cached path if it still exists.
        if let Some(path) = &self.cached_codelldb_path {
            if std::fs::metadata(path).map_or(false, |stat| stat.is_file()) {
                return Ok((path.clone(), Vec::new()));
            }
        }

        let (platform, arch) = zed::current_platform();
        let platform_str = match platform {
            zed::Os::Mac => "darwin",
            zed::Os::Linux => "linux",
            zed::Os::Windows => "win32",
        };
        let arch_str = match arch {
            zed::Architecture::Aarch64 => "arm64",
            zed::Architecture::X8664 => "x64",
            zed::Architecture::X86 => {
                return Err("32-bit x86 is not supported by CodeLLDB".to_string());
            }
        };
        let asset_name = format!("codelldb-{platform_str}-{arch_str}.vsix");
        let exe_suffix = match platform {
            zed::Os::Windows => ".exe",
            _ => "",
        };

        let release = zed::latest_github_release(
            "vadimcn/codelldb",
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let asset = release
            .assets
            .iter()
            .find(|a| a.name == asset_name)
            .ok_or_else(|| {
                format!(
                    "no CodeLLDB asset found matching {:?}. Available: {:?}",
                    asset_name,
                    release.assets.iter().map(|a| &a.name).collect::<Vec<_>>()
                )
            })?;

        let version_dir = format!("codelldb-{}", release.version);
        let binary_name = format!("codelldb{exe_suffix}");
        let binary_path = format!("{version_dir}/extension/adapter/{binary_name}");

        if !std::fs::metadata(&binary_path).map_or(false, |stat| stat.is_file()) {
            zed::download_file(&asset.download_url, &version_dir, zed::DownloadedFileType::Zip)
                .map_err(|e| format!("failed to download CodeLLDB: {e}"))?;

            if platform != zed::Os::Windows {
                zed::make_file_executable(&binary_path)?;
            }
        }

        self.cached_codelldb_path = Some(binary_path.clone());
        Ok((binary_path, Vec::new()))
    }

    /// Resolve the absolute output path of an xmake target by running
    /// `xmake l targetpath.lua <target> <projectdir>` through Zed's WIT-backed
    /// process API.
    ///
    /// Returns `None` (rather than an error) when the project is not configured
    /// yet or no binary target exists, so callers can fall back gracefully.
    fn resolve_target_program(
        &self,
        target_name: &str,
        project_dir: &str,
    ) -> Option<String> {
        let script_path = utils::ensure_target_path_script()?;
        let script_str = script_path.to_str()?.to_string();

        // NOTE: `zed::Command` (the process WIT) has no `cwd` field, so the
        // project directory is passed as an argument and the lua script does
        // `os.cd` into it. Using `std::process::Command` here would silently
        // fail inside the WASM sandbox, which is why the previous debug
        // support never resolved a program path.
        let mut cmd = zed::Command::new("xmake")
            .arg("l")
            .arg(script_str)
            .arg(target_name.to_string())
            .arg(project_dir.to_string());

        let output = cmd.output().ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        utils::parse_target_path_output(&stdout)
    }

    /// Extract the xmake target name from a build/run task.
    ///
    /// For tasks like `xmake run foo` / `xmake build foo` the target is
    /// `args[1]`. For tasks without an explicit target it falls back to
    /// `"default"`, which matches xmake's own behaviour.
    ///
    /// Flags placed where a target would be (e.g. the shipped
    /// `xmake build -a` / `xmake build -v` tasks, whose `args[1]` is `-a` or
    /// `-v`) are not target names and must not be treated as such — the
    /// locator would otherwise try to resolve a target literally named `-a`.
    fn target_name_from_task(task: &zed_extension_api::TaskTemplate) -> String {
        if task.command == "xmake" {
            if let Some(name) = task.args.get(1) {
                if !name.is_empty() && !name.starts_with('-') {
                    return name.clone();
                }
            }
        }
        "default".to_string()
    }
}

impl zed::Extension for XMakeExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
            cached_codelldb_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<zed::Command> {
        let settings = LspSettings::for_worktree("xmake-ls", worktree)
            .ok()
            .and_then(|lsp_settings| lsp_settings.binary)
            .and_then(|binary_settings| binary_settings.path);

        if let Some(path) = settings {
            return Ok(zed::Command {
                command: path,
                args: vec![],
                env: Default::default(),
            });
        }

        if let Some(path) = worktree.which("xmake_ls") {
            self.cached_binary_path = Some(path.clone());
            return Ok(zed::Command {
                command: path,
                args: vec![],
                env: Default::default(),
            });
        }

        if let Some(path) = &self.cached_binary_path {
            if std::fs::metadata(path).map_or(false, |stat| stat.is_file()) {
                return Ok(zed::Command {
                    command: path.clone(),
                    args: vec![],
                    env: Default::default(),
                });
            }
        }

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );

        let release = zed::latest_github_release(
            "CppCXY/xmake_ls",
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let (platform, arch) = zed::current_platform();

        let (asset_name, file_type, binary_name) = match platform {
            zed::Os::Mac => {
                let arch_str = match arch {
                    zed::Architecture::Aarch64 => "arm64",
                    zed::Architecture::X8664 => "x64",
                    zed::Architecture::X86 => {
                        return Err("32-bit macOS is not supported by xmake_ls".to_string());
                    }
                };
                (
                    format!("xmake_ls-darwin-{}.tar.gz", arch_str),
                    zed::DownloadedFileType::GzipTar,
                    "xmake_ls".to_string(),
                )
            }
            zed::Os::Linux => {
                let variant = self.get_linux_variant(worktree)?;
                (
                    format!("xmake_ls-linux-{}.tar.gz", variant),
                    zed::DownloadedFileType::GzipTar,
                    "xmake_ls".to_string(),
                )
            }
            zed::Os::Windows => {
                let arch_str = match arch {
                    zed::Architecture::Aarch64 => "arm64",
                    zed::Architecture::X8664 => "x64",
                    zed::Architecture::X86 => "ia32",
                };
                (
                    format!("xmake_ls-win32-{}.zip", arch_str),
                    zed::DownloadedFileType::Zip,
                    "xmake_ls.exe".to_string(),
                )
            }
        };

        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| {
                format!(
                    "no asset found matching {:?}. Available: {:?}",
                    asset_name,
                    release.assets.iter().map(|a| &a.name).collect::<Vec<_>>()
                )
            })?;

        let version_dir = format!("xmake_ls-{}", release.version);
        let binary_path = format!("{version_dir}/{binary_name}");

        if !std::fs::metadata(&binary_path).map_or(false, |stat| stat.is_file()) {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );

            zed::download_file(&asset.download_url, &version_dir, file_type)
                .map_err(|e| format!("failed to download file: {e}"))?;

            let entries =
                std::fs::read_dir(".").map_err(|e| format!("failed to list working directory {e}"))?;
            for entry in entries {
                let entry = entry.map_err(|e| format!("failed to load directory entry {e}"))?;
                if entry.file_name().to_str() != Some(&version_dir) {
                    std::fs::remove_dir_all(entry.path()).ok();
                }
            }

            if platform != zed::Os::Windows {
                zed::make_file_executable(&binary_path)?;
            }
        }

        self.cached_binary_path = Some(binary_path.clone());
        Ok(zed::Command {
            command: binary_path,
            args: vec![],
            env: Default::default(),
        })
    }

    fn language_server_initialization_options(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Option<zed::serde_json::Value>> {
        let settings = self.get_settings(worktree)?;

        let init_options = zed::serde_json::json!({
            "settings": settings.clone().unwrap_or_else(|| zed::serde_json::json!({}))
        });

        Ok(Some(init_options))
    }

    fn language_server_workspace_configuration(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Option<zed::serde_json::Value>> {
        let settings = self.get_settings(worktree)?;
        Ok(settings)
    }

    fn get_dap_binary(
        &mut self,
        adapter_name: String,
        config: zed_extension_api::DebugTaskDefinition,
        user_provided_debug_adapter_path: Option<String>,
        worktree: &Worktree,
    ) -> Result<zed_extension_api::DebugAdapterBinary, String> {
        if adapter_name != "xmake" {
            return Err(format!("This adapter does not support: {}", adapter_name));
        }

        let xmake_config: XMakeDebugConfig = serde_json::from_str(&config.config)
            .map_err(|e| format!("Failed to parse debug config: {}", e))?;

        let debugger_type = xmake_config.debugger.as_deref().unwrap_or("codelldb");
        let (debugger_path, base_args) = match debugger_type {
            "codelldb" => {
                let path_and_base_args = self.find_codelldb(worktree)?;
                (path_and_base_args.0, path_and_base_args.1)
            }
            "lldb-dap" => {
                let path_and_base_args = self.find_lldb_dap(worktree)?;
                (
                    path_and_base_args.0,
                    if let Some(basearg) = path_and_base_args.1 {
                        vec![basearg]
                    } else {
                        vec![]
                    },
                )
            }
            "gdb-dap" => {
                let path_and_base_args = self.find_gdb_dap(worktree)?;
                (path_and_base_args.0, path_and_base_args.1)
            }
            _ => return Err(format!("Unsupported debugger: {}", debugger_type)),
        };

        let request = match xmake_config.request.as_str() {
            "launch" => zed_extension_api::StartDebuggingRequestArgumentsRequest::Launch,
            "attach" => zed_extension_api::StartDebuggingRequestArgumentsRequest::Attach,
            other => return Err(format!("Invalid request type: {}", other)),
        };

        let (command, arguments) = user_provided_debug_adapter_path
            .map(|path| (path, Vec::<String>::new()))
            .or_else(|| Some((debugger_path, base_args)))
            .ok_or_else(|| "Could not find debugger path".to_owned())?;

        // Build the adapter-specific configuration. The underlying debuggers
        // (lldb-dap / gdb-dap) do not understand the xmake wrapper fields, so
        // we translate to their own schema here.
        let adapter_config = xmake_config.to_adapter_config();
        let configuration = serde_json::to_string(&adapter_config)
            .map_err(|e| format!("failed to serialize adapter config: {e}"))?;

        Ok(zed_extension_api::DebugAdapterBinary {
            command: Some(command),
            arguments,
            envs: xmake_config.env.into_iter().collect(),
            cwd: Some(
                xmake_config
                    .cwd
                    .clone()
                    .unwrap_or_else(|| worktree.root_path()),
            ),
            connection: None,
            request_args: zed_extension_api::StartDebuggingRequestArguments {
                configuration,
                request,
            },
        })
    }

    fn dap_request_kind(
        &mut self,
        _adapter_name: String,
        _config: zed_extension_api::serde_json::Value,
    ) -> Result<zed_extension_api::StartDebuggingRequestArgumentsRequest, String> {
        if let Some(request_str) = _config.get("request").and_then(|v| v.as_str()) {
            match request_str {
                "launch" => Ok(zed_extension_api::StartDebuggingRequestArgumentsRequest::Launch),
                "attach" => Ok(zed_extension_api::StartDebuggingRequestArgumentsRequest::Attach),
                other => Err(format!("Invalid request type: {}", other)),
            }
        } else {
            Ok(zed_extension_api::StartDebuggingRequestArgumentsRequest::Launch)
        }
    }

    fn dap_config_to_scenario(
        &mut self,
        config: zed_extension_api::DebugConfig,
    ) -> Result<zed_extension_api::DebugScenario, String> {
        match config.request {
            zed_extension_api::DebugRequest::Launch(launch) => {
                let xmake_config = XMakeDebugConfig {
                    program: Some(launch.program),
                    args: Some(launch.args),
                    cwd: launch.cwd.clone(),
                    env: launch.envs.into_iter().collect(),
                    request: "launch".to_owned(),
                    stop_at_entry: config.stop_on_entry,
                    pid: None,
                    debugger: Some("codelldb".to_string()),
                    console: Some("integratedTerminal".to_string()),
                    label: "xmake debug".to_string(),
                };
                let config_json = serde_json::to_string(&xmake_config)
                    .map_err(|e| format!("failed to serialize debug config: {e}"))?;
                Ok(zed_extension_api::DebugScenario {
                    adapter: config.adapter,
                    label: config.label,
                    config: config_json,
                    tcp_connection: None,
                    build: None,
                })
            }
            zed_extension_api::DebugRequest::Attach(attach) => {
                let xmake_config = XMakeDebugConfig {
                    program: None,
                    args: None,
                    cwd: None,
                    env: Default::default(),
                    request: "attach".to_owned(),
                    stop_at_entry: config.stop_on_entry,
                    pid: attach.process_id,
                    debugger: None,
                    console: None,
                    label: "xmake debug".to_string(),
                };
                let config_json = serde_json::to_string(&xmake_config)
                    .map_err(|e| format!("failed to serialize debug config: {e}"))?;
                Ok(zed_extension_api::DebugScenario {
                    label: config.label,
                    adapter: config.adapter,
                    config: config_json,
                    tcp_connection: None,
                    build: None,
                })
            }
        }
    }

    fn dap_locator_create_scenario(
        &mut self,
        _locator_name: String,
        build_task: zed_extension_api::TaskTemplate,
        resolved_label: String,
        debug_adapter_name: String,
    ) -> Option<zed_extension_api::DebugScenario> {
        // Zed feeds EVERY available task to every registered locator, so we
        // must reject non-xmake tasks early (the docs explicitly recommend
        // this). We only know how to debug tasks that build or run an xmake
        // target, i.e. `xmake build [target]` or `xmake run [target]`.
        //
        // Anything else — including shell-chained commands such as the old
        // `sh -c "xmake build && xmake run"` tasks, whose args[0] is "-c" —
        // is rejected here on purpose, because we cannot reliably extract a
        // target from them. Users who want to debug those should use a manual
        // `.zed/debug.json`.
        if build_task.command != "xmake" {
            return None;
        }
        let subcommand = build_task.args.get(0).map(String::as_str).unwrap_or("");
        let is_build = subcommand == "build";
        let is_run = subcommand == "run";
        if !is_build && !is_run {
            return None;
        }

        // The build task's cwd is the project root. At this stage the task
        // template is *unresolved* (variables like $ZED_WORKTREE_ROOT are not
        // substituted yet), so we cannot run xmake to resolve the program
        // path here — that happens in `run_dap_locator` after the build step,
        // when Zed passes us a resolved task. We still record the cwd so the
        // build step runs in the right directory.
        let project_root = build_task.cwd.clone()?;
        let target_name = Self::target_name_from_task(&build_task);
        // "default" is our sentinel for "no explicit target was given".
        // It is *not* a real xmake target name (`xmake build default` fails),
        // so when it appears we must build without a target argument and let
        // `run_dap_locator` resolve the first default binary target instead.
        let is_default_target = target_name == "default";

        let xmake_config = XMakeDebugConfig {
            // Left empty on purpose: resolved post-build by run_dap_locator.
            program: None,
            // The build task's args are xmake build flags (e.g.
            // `["build", "<target>"]`), not arguments for the debugged
            // program. Leave program args empty; users who need runtime
            // arguments should use a manual debug.json.
            args: None,
            cwd: Some(project_root),
            env: build_task.env.clone().into_iter().collect(),
            request: "launch".to_string(),
            stop_at_entry: Some(false),
            pid: None,
            debugger: Some("codelldb".to_string()),
            console: Some("integratedTerminal".to_string()),
            label: resolved_label.clone(),
        };
        let config_json = serde_json::to_string(&xmake_config).ok()?;

        // Build step: `xmake build [<target>]`. After it completes, Zed calls
        // `run_dap_locator` (because `locator_name` is set) to resolve the
        // freshly built executable.
        let build_label = if is_default_target {
            "xmake build".to_string()
        } else {
            format!("xmake build {}", target_name)
        };
        let mut build_args = vec!["build".to_string()];
        if !is_default_target {
            build_args.push(target_name.clone());
        }
        let build_template = zed_extension_api::TaskTemplate {
            label: build_label,
            command: "xmake".to_string(),
            args: build_args,
            env: Default::default(),
            cwd: build_task.cwd.clone(),
        };
        let build_task_def = zed_extension_api::BuildTaskDefinition::Template(
            zed_extension_api::BuildTaskDefinitionTemplatePayload {
                locator_name: Some("xmake".to_string()),
                template: build_template,
            },
        );

        Some(zed_extension_api::DebugScenario {
            adapter: debug_adapter_name,
            label: resolved_label,
            config: config_json,
            tcp_connection: None,
            build: Some(build_task_def),
        })
    }

    fn run_dap_locator(
        &mut self,
        _locator_name: String,
        config: zed_extension_api::TaskTemplate,
    ) -> Result<zed_extension_api::DebugRequest, String> {
        // `config` is the *resolved* build task (variables substituted) that
        // just ran. Its cwd is the project root and its args tell us which
        // target was built.
        let project_root = config
            .cwd
            .clone()
            .ok_or_else(|| "build task has no cwd; cannot locate xmake project".to_string())?;
        let target_name = Self::target_name_from_task(&config);
        let display_target = if target_name == "default" {
            "default target".to_string()
        } else {
            format!("target `{}`", target_name)
        };

        let program = self
            .resolve_target_program(&target_name, &project_root)
            .ok_or_else(|| {
                format!(
                    "could not resolve the executable for {} in `{}`. \
                     Run `xmake config -m debug` and `xmake build` first.",
                    display_target, project_root
                )
            })?;

        Ok(zed_extension_api::DebugRequest::Launch(
            zed_extension_api::LaunchRequest {
                program,
                cwd: Some(project_root),
                // Runtime args for the debugged program are not known to the
                // locator (they belong to `xmake run`, not `xmake build`).
                // Pass none; users needing args should use a manual debug.json.
                args: Vec::new(),
                envs: config.env.clone().into_iter().collect(),
            },
        ))
    }
}

zed::register_extension!(XMakeExtension);
