#!/usr/bin/env python3
"""Generate languages/xmake/tasks.json and tasks/project-tasks.json.

This script is the single source of truth for the tasks shipped by the
xmake Zed extension. It regenerates two files:

1. `languages/xmake/tasks.json` — the language-scoped tasks Zed shows while an
   `xmake.lua` file is open (the extension API only supports tasks registered
   per language; there is no extension-level task registration in
   zed_extension_api).

2. `tasks/project-tasks.json` — a committable template of the *core* tasks.
   Copy it to `.zed/tasks.json` (or run one of the two installer tasks below)
   to make the tasks available project-wide from any file, regardless of which
   file is open. This mirrors the VS Code xmake extension, whose build/run
   actions are workspace-scoped rather than tied to the active editor tab.

Cross-platform notes (fixes upstream issues #3, #4 and #5):

* Every task's `command`/`args` is a plain `xmake` invocation, so the tasks run
  on Windows, macOS and Linux without a POSIX shell:
  - `xmake build -r`  == clean + build (replaces `sh -c "xmake clean -a && xmake build"`)
  - `xmake run`       builds the target automatically before running
    (replaces `sh -c "xmake build && xmake run"`)
  - `xmake project -k compile_commands .zed` writes `compile_commands.json`
    straight into `.zed/` (replaces `sh -c "... && mkdir -p .zed && mv ..."`)
* The two "install project tasks" tasks are the only shell-dependent tasks:
  - POSIX variant uses `sh` (macOS, Linux, Git Bash on Windows)
  - Windows variant uses `powershell.exe` (Windows 10/11, no shell needed)
"""

import json

# ---------------------------------------------------------------------------
# Core tasks written into the project's `.zed/tasks.json` by the installer
# tasks (and shipped as tasks/project-tasks.json). Only plain `xmake`
# invocations so they work on every platform.
#
# NOTE: `cwd` is deliberately OMITTED. Zed substitutes `$ZED_*` variables in
# task `args` *before* the shell runs, so a `$ZED_WORKTREE_ROOT` inside the
# payload would be baked in as a hard-coded absolute path. Omitting `cwd`
# makes Zed default to the worktree root, which is exactly what we want, and
# keeps the generated file portable / committable.
PROJECT_TASKS = [
    {"label": "xmake: build", "command": "xmake", "args": ["build"], "reveal": "always"},
    {"label": "xmake: build all targets", "command": "xmake", "args": ["build", "-a"], "reveal": "always"},
    {"label": "xmake: rebuild", "command": "xmake", "args": ["build", "-r"], "reveal": "always"},
    {"label": "xmake: clean", "command": "xmake", "args": ["clean"]},
    {"label": "xmake: run", "command": "xmake", "args": ["run"], "reveal": "always"},
    {"label": "xmake: configure", "command": "xmake", "args": ["config"]},
    {"label": "xmake: debug mode", "command": "xmake", "args": ["config", "-m", "debug"]},
    {"label": "xmake: release mode", "command": "xmake", "args": ["config", "-m", "release"]},
    {"label": "xmake: generate compile_commands.json (into .zed)",
     "command": "xmake", "args": ["project", "-k", "compile_commands", ".zed"],
     "reveal": "always"},
]

# Compact JSON for the embedded project tasks payload (no whitespace).
project_tasks_json = json.dumps(PROJECT_TASKS, separators=(",", ":"))


def posix_install_script(payload: str) -> str:
    """POSIX shell script that writes the project tasks into .zed/tasks.json.

    Uses a quoted heredoc so no shell expansion happens on the JSON body.
    Backs up an existing .zed/tasks.json to .zed/tasks.json.bak first.
    """
    return "\n".join([
        'set -e',
        'mkdir -p .zed',
        'F=".zed/tasks.json"',
        'if [ -f "$F" ]; then cp "$F" "$F.bak"; echo "Backed up existing tasks.json to tasks.json.bak"; fi',
        "cat > \"$F\" <<'XMAKE_TASKS_EOF'",
        payload,
        "XMAKE_TASKS_EOF",
        'echo "Installed xmake tasks to $F - they are now available from any file via task: spawn."',
    ])


def powershell_install_script(payload: str) -> str:
    """Windows PowerShell script that writes the project tasks.

    The JSON payload is embedded as a PowerShell single-quoted string, so the
    double quotes inside the JSON need no escaping (and the JSON contains no
    single quotes). Written as UTF-8 without BOM.
    """
    # PowerShell single-quoted strings escape a literal quote by doubling it.
    ps_payload = payload.replace("'", "''")
    return (
        "$ErrorActionPreference = 'Stop'; "
        "$d = Join-Path (Get-Location) '.zed'; "
        "New-Item -ItemType Directory -Force -Path $d | Out-Null; "
        "$f = Join-Path $d 'tasks.json'; "
        "if (Test-Path $f) { Copy-Item $f ($f + '.bak') -Force; Write-Output 'Backed up existing tasks.json to tasks.json.bak' }; "
        "[System.IO.File]::WriteAllText($f, '" + ps_payload + "', (New-Object System.Text.UTF8Encoding($false))); "
        "Write-Output ('Installed xmake tasks to ' + $f + ' - they are now available from any file via task: spawn.')"
    )


INSTALL_TASKS = [
    {
        "label": "xmake: install project tasks (POSIX: macOS/Linux/Git Bash)",
        "command": "sh",
        "args": ["-c", posix_install_script(project_tasks_json)],
        "cwd": "$ZED_WORKTREE_ROOT",
        "reveal": "always",
    },
    {
        "label": "xmake: install project tasks (Windows PowerShell)",
        "command": "powershell",
        "args": ["-NoProfile", "-Command", powershell_install_script(project_tasks_json)],
        "cwd": "$ZED_WORKTREE_ROOT",
        "reveal": "always",
    },
]

# ---------------------------------------------------------------------------
# Full language task list: core project tasks + the remaining xmake workflow
# tasks, all cross-platform (plain `xmake` invocations).
LANGUAGE_TASKS = PROJECT_TASKS + [
    {"label": "xmake: build (verbose)", "command": "xmake", "args": ["build", "-v"],
     "tags": ["xmake-build"], "reveal": "always"},
    {"label": "xmake: clean all", "command": "xmake", "args": ["clean", "-a"]},
    {"label": "xmake: build & run", "command": "xmake", "args": ["run"], "reveal": "always"},
    {"label": "xmake: build & run (verbose)", "command": "xmake", "args": ["run", "-v"],
     "reveal": "always"},
    {"label": "xmake: configure", "command": "xmake", "args": ["config"]},
    {"label": "xmake: clean configuration", "command": "xmake", "args": ["config", "-c"]},
    {"label": "xmake: configure menu", "command": "xmake", "args": ["config", "--menu"]},
    {"label": "xmake: debug mode", "command": "xmake", "args": ["config", "-m", "debug"]},
    {"label": "xmake: release mode", "command": "xmake", "args": ["config", "-m", "release"]},
    {"label": "xmake: relwithdebinfo mode", "command": "xmake", "args": ["config", "-m", "relwithdebinfo"]},
    {"label": "xmake: minsizerel mode", "command": "xmake", "args": ["config", "-m", "minsizerel"]},
    {"label": "xmake: generate Visual Studio project", "command": "xmake",
     "args": ["project", "-k", "vs2022"]},
    {"label": "xmake: generate Xcode project", "command": "xmake",
     "args": ["project", "-k", "xcode"]},
    {"label": "xmake: install", "command": "xmake", "args": ["install", "-o", "install"]},
    {"label": "xmake: package", "command": "xmake", "args": ["package"]},
    {"label": "xmake: format", "command": "xmake", "args": ["format"]},
    {"label": "xmake: show targets", "command": "xmake", "args": ["show", "-l"], "reveal": "always"},
    {"label": "xmake: show configuration", "command": "xmake", "args": ["show"], "reveal": "always"},
    {"label": "xmake: version", "command": "xmake", "args": ["--version"], "reveal": "always"},
    {"label": "xmake: GCC toolchain", "command": "xmake",
     "args": ["config", "--toolchain=gcc", "-y"]},
    {"label": "xmake: Clang toolchain", "command": "xmake",
     "args": ["config", "--toolchain=clang", "-y"]},
    {"label": "xmake: MSVC toolchain (VS 2022)", "command": "xmake",
     "args": ["config", "--toolchain=msvc", "--vs=2022", "-y"]},
    {"label": "xmake: Zig toolchain", "command": "xmake",
     "args": ["config", "--toolchain=@zig", "-y"]},
    {"label": "xmake: C++20 standard", "command": "xmake",
     "args": ["config", "--cxxstd=c++20", "-y"]},
    {"label": "xmake: C++23 standard", "command": "xmake",
     "args": ["config", "--cxxstd=c++23", "-y"]},
    {"label": "xmake: enable LTO", "command": "xmake", "args": ["config", "--enable-lto", "-y"]},
    {"label": "xmake: enable ASAN", "command": "xmake", "args": ["config", "--enable-asan", "-y"]},
    {"label": "xmake: enable TSAN", "command": "xmake", "args": ["config", "--enable-tsan", "-y"]},
    {"label": "xmake: enable UBSAN", "command": "xmake", "args": ["config", "--enable-ubsan", "-y"]},
    {"label": "xmake: build with Mold linker", "command": "xmake",
     "args": ["config", "--ld=mold", "-y"]},
    {"label": "xmake: build with LLD linker", "command": "xmake",
     "args": ["config", "--ld=lld", "-y"]},
    {"label": "xmake: create project (help)", "command": "xmake",
     "args": ["create", "--help"], "reveal": "always"},
    {"label": "xmake: create C++ console project", "command": "xmake",
     "args": ["create", "-l", "c++", "-t", "console", "${ZED_SELECTED_TEXT:myproject}"]},
    {"label": "xmake: create C console project", "command": "xmake",
     "args": ["create", "-l", "c", "-t", "console", "${ZED_SELECTED_TEXT:myproject}"]},
    {"label": "xmake: create Rust console project", "command": "xmake",
     "args": ["create", "-l", "rust", "-t", "console", "${ZED_SELECTED_TEXT:myproject}"]},
    {"label": "xmake: create C++ static library project", "command": "xmake",
     "args": ["create", "-l", "c++", "-t", "static", "${ZED_SELECTED_TEXT:myproject}"]},
    {"label": "xmake: create C++ shared library project", "command": "xmake",
     "args": ["create", "-l", "c++", "-t", "shared", "${ZED_SELECTED_TEXT:myproject}"]},
]

# The first few core tasks keep their runnable tags (used by the debug
# locator to offer "Debug: ..." scenarios and by inline runnables).
TAG_CORE = {"xmake: build": "xmake-build", "xmake: run": "xmake-run"}
for task in LANGUAGE_TASKS:
    tag = TAG_CORE.get(task["label"])
    if tag:
        task.setdefault("tags", []).insert(0, tag)

# De-duplicate labels (the core list already contains configure/debug mode etc.,
# so drop the later duplicates).
LANGUAGE_TASKS = list(
    {task["label"]: task for task in LANGUAGE_TASKS}.values()
)

tasks = LANGUAGE_TASKS + INSTALL_TASKS

with open("languages/xmake/tasks.json", "w", encoding="utf-8") as f:
    json.dump(tasks, f, indent=2, ensure_ascii=False)
    f.write("\n")

with open("tasks/project-tasks.json", "w", encoding="utf-8") as f:
    json.dump(PROJECT_TASKS, f, indent=2, ensure_ascii=False)
    f.write("\n")

print(f"Wrote languages/xmake/tasks.json with {len(tasks)} tasks")
print(f"Wrote tasks/project-tasks.json with {len(PROJECT_TASKS)} tasks")
