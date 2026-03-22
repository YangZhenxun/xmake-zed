use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};
use zed_extension_api::{self as zed, LanguageServerId, Result, Worktree, settings::LspSettings};
mod utils;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
struct XMakeDebugConfig {
    program: Option<String>,
    args: Option<Vec<String>>,
    cwd: Option<String>,
    env: HashMap<String, String>,
    request: String,
    stop_at_entry: Option<bool>,
    pid: Option<u32>,
    debugger: Option<String>,
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
        }
    }
}

struct XMakeExtension {
    cached_binary_path: Option<String>,
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
                "sudo apt install lldb` (Ubuntu/Debian) or `sudo pacman -S lldb` (Arch)."
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

    fn read_default_target(&self, project_root: String) -> Option<String> {
        let settings_path = Path::new(project_root.as_str())
            .join(".zed")
            .join("settings.json");
        if !settings_path.exists() {
            return None;
        }
        let content = fs::read_to_string(settings_path).ok()?;
        let json: serde_json::Value = serde_json::from_str(&content).ok()?;
        json.get("xmake.defaultTarget")?.as_str().map(String::from)
    }

    fn get_target_name(&self, build_task: zed_extension_api::TaskTemplate) -> String {
        let project_root = build_task.cwd.as_deref();
        let config_target = if let Some(some_project_root) = project_root {
            self.read_default_target(some_project_root.to_string())
        } else {
            None
        };
        let target_name = config_target.unwrap_or_else(|| "default".to_string());
        return target_name;
    }
}

impl zed::Extension for XMakeExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
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
            //let path_str = path.to_string_lossy().to_string();
            self.cached_binary_path = Some(path.clone());
            return Ok(zed::Command {
                command: path,
                args: vec![],
                env: Default::default(),
            });
        }

        if let Some(path) = &self.cached_binary_path {
            if fs::metadata(path).map_or(false, |stat| stat.is_file()) {
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

        if !fs::metadata(&binary_path).map_or(false, |stat| stat.is_file()) {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );

            zed::download_file(&asset.download_url, &version_dir, file_type)
                .map_err(|e| format!("failed to download file: {e}"))?;

            let entries =
                fs::read_dir(".").map_err(|e| format!("failed to list working directory {e}"))?;
            for entry in entries {
                let entry = entry.map_err(|e| format!("failed to load directory entry {e}"))?;
                if entry.file_name().to_str() != Some(&version_dir) {
                    fs::remove_dir_all(entry.path()).ok();
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
        let configuration = config.config.to_string();
        let xmake_config: XMakeDebugConfig = serde_json::from_str(&config.config)
            .map_err(|e| format!("Failed to parse debug config: {}", e))?;
        let debugger_type = xmake_config.debugger.as_deref().unwrap_or("lldb-dap");
        let (debugger_path, base_args) = match debugger_type {
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
            _ => return Err(format!("Invalid request type: {}", xmake_config.request)),
        };
        let (command, arguments) = user_provided_debug_adapter_path
            .map(|path| (path, Vec::<String>::new()))
            .or_else(|| Some((debugger_path, base_args)))
            .ok_or_else(|| "Could not find debugger path".to_owned())?;
        Ok(zed_extension_api::DebugAdapterBinary {
            command: Some(command),
            arguments,
            envs: xmake_config.env.into_iter().collect(),
            cwd: Some(xmake_config.cwd.unwrap_or_else(|| worktree.root_path())),
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
                let xmake_config = serde_json::to_string(&XMakeDebugConfig {
                    program: Some(launch.program),
                    args: Some(launch.args),
                    cwd: launch.cwd.clone(),
                    env: launch.envs.into_iter().collect(),
                    request: "launch".to_owned(),
                    stop_at_entry: config.stop_on_entry,
                    pid: None,
                    debugger: None,
                })
                .unwrap();
                Ok(zed_extension_api::DebugScenario {
                    adapter: config.adapter,
                    label: config.label,
                    config: xmake_config,
                    tcp_connection: None,
                    build: None,
                })
            }
            zed_extension_api::DebugRequest::Attach(attach) => {
                let xmake_config = serde_json::to_string(&XMakeDebugConfig {
                    program: None,
                    args: None,
                    cwd: None,
                    env: Default::default(),
                    request: "attach".to_owned(),
                    stop_at_entry: config.stop_on_entry,
                    pid: attach.process_id,
                    debugger: None,
                })
                .unwrap();
                Ok(zed::DebugScenario {
                    label: config.label,
                    adapter: config.adapter,
                    config: xmake_config,
                    tcp_connection: None,
                    build: None,
                })
            }
        }
    }

    fn dap_locator_create_scenario(
        &mut self,
        _locator_name: String,
        _build_task: zed_extension_api::TaskTemplate,
        _resolved_label: String,
        _debug_adapter_name: String,
    ) -> Option<zed_extension_api::DebugScenario> {
        // Get the project root (cwd)
        let cwd = _build_task.cwd.as_ref()?;

        // Determine the target name:
        // - If the task command is "xmake", extract target from args (e.g., "xmake run foo" -> "foo")
        // - Otherwise, try to find a default binary target
        let target_name = if _build_task.command == "xmake" {
            _build_task
                .args
                .get(1)
                .cloned()
                .unwrap_or_else(|| "default".to_string())
        } else {
            // For non-xmake tasks, try to find any binary target
            "default".to_string()
        };

        // Get the target program path by running xmake
        let get_target_path_script = utils::get_assets_script_path("targetpath.lua".to_string());
        let program_path = if let Some(target_path_script) = get_target_path_script {
            if let Some(cwd) = &_build_task.cwd {
                let output = std::process::Command::new("xmake")
                    .args(&["l", &target_path_script.to_str()?, &target_name])
                    .current_dir(cwd)
                    .output()
                    .ok()?;

                // Parse the output to get the actual path (between __begin__ and __end__)
                let output_str = String::from_utf8_lossy(&output.stdout);
                let mut in_section = false;
                let mut path = String::new();
                for line in output_str.lines() {
                    if line == "__begin__" {
                        in_section = true;
                        continue;
                    }
                    if line == "__end__" {
                        break;
                    }
                    if in_section {
                        path = line.to_string();
                    }
                }
                if !path.is_empty() { Some(path) } else { None }
            } else {
                None
            }
        } else {
            None
        };

        let xmake_config = XMakeDebugConfig {
            program: program_path,
            args: Some(_build_task.args),
            cwd: _build_task.cwd.clone(),
            env: _build_task.env.into_iter().collect(),
            request: "launch".to_string(),
            stop_at_entry: Some(false),
            pid: None,
            debugger: Some("lldb-dap".to_string()),
        };

        let config_json = serde_json::to_string(&xmake_config).ok()?;

        let build_template = zed_extension_api::TaskTemplate {
            label: format!("Build {}", target_name),
            command: "xmake".to_string(),
            args: vec!["build".to_string(), target_name.clone()],
            env: Default::default(),
            cwd: _build_task.cwd.clone(),
        };

        let build_task = zed_extension_api::BuildTaskDefinition::Template(
            zed_extension_api::BuildTaskDefinitionTemplatePayload {
                locator_name: Some("xmake".to_string()),
                template: build_template,
            },
        );

        Some(zed_extension_api::DebugScenario {
            adapter: _debug_adapter_name,
            label: _resolved_label,
            config: config_json,
            tcp_connection: None,
            build: Some(build_task),
        })
    }
}

zed::register_extension!(XMakeExtension);
