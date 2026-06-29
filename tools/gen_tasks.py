#!/usr/bin/env python3
"""Generate languages/xmake/tasks.json.

The file is mostly hand-written Zed tasks, plus one generated task
"xmake: install project tasks (to .zed/tasks.json)" that writes a core set of
xmake tasks into the project's .zed/tasks.json so they are available
project-wide regardless of which file is open (fixes upstream issue #4).

We generate the file with a script because the install task embeds a JSON
document inside a shell heredoc inside a JSON string, which is painful to
hand-escape correctly.
"""
import json

# Core tasks that get written into the project's .zed/tasks.json by the
# "install project tasks" task. Kept small and high-value on purpose.
#
# NOTE: `cwd` is deliberately OMITTED here. Zed substitutes `$ZED_*` variables
# in task `args` *before* the shell runs, so a `$ZED_WORKTREE_ROOT` inside the
# quoted heredoc body would be baked in as a hard-coded absolute path. Omitting
# `cwd` makes Zed default to the worktree root, which is exactly what we want,
# and keeps the generated file portable / committable.
PROJECT_TASKS = [
    {"label": "xmake: build", "command": "xmake", "args": ["build"], "reveal": "always"},
    {"label": "xmake: build all targets", "command": "xmake", "args": ["build", "-a"], "reveal": "always"},
    {"label": "xmake: rebuild (clean + build)", "command": "sh",
     "args": ["-c", "xmake clean -a && xmake build"], "reveal": "always"},
    {"label": "xmake: clean", "command": "xmake", "args": ["clean"]},
    {"label": "xmake: run", "command": "xmake", "args": ["run"], "reveal": "always"},
    {"label": "xmake: build & run", "command": "sh",
     "args": ["-c", "xmake build && xmake run"], "reveal": "always"},
    {"label": "xmake: configure", "command": "xmake", "args": ["config"]},
    {"label": "xmake: debug mode", "command": "xmake", "args": ["config", "-m", "debug"]},
    {"label": "xmake: generate compile_commands.json (into .zed)", "command": "sh",
     "args": ["-c",
              "xmake project -k compile_commands && mkdir -p .zed && "
              "mv -f compile_commands.json .zed/ 2>/dev/null; true"],
     "reveal": "always"},
]

# Compact JSON for the embedded project tasks file (no leading/trailing whitespace).
project_tasks_json = json.dumps(PROJECT_TASKS, separators=(",", ":"))

# Shell script the install task runs. Writes .zed/tasks.json, backing up any
# existing file. Uses a quoted heredoc so no shell expansion happens on the
# JSON body (it contains $ZED_WORKTREE_ROOT which must be preserved literally).
install_script = (
    'mkdir -p "$ZED_WORKTREE_ROOT/.zed"\n'
    'F="$ZED_WORKTREE_ROOT/.zed/tasks.json"\n'
    'if [ -f "$F" ]; then cp "$F" "$F.bak"; echo "Backed up existing tasks.json to tasks.json.bak"; fi\n'
    "cat > \"$F\" <<'XMAKE_TASKS_EOF'\n"
    + project_tasks_json
    + "\nXMAKE_TASKS_EOF\n"
    'echo "Installed xmake tasks to $F — they are now available from any file via task: spawn."'
)

install_task = {
    "label": "xmake: install project tasks (to .zed/tasks.json)",
    "command": "sh",
    "args": ["-c", install_script],
    "cwd": "$ZED_WORKTREE_ROOT",
    "reveal": "always",
}

# Load the hand-written tasks (already on disk) and append the generated one.
with open("languages/xmake/tasks.json", "r", encoding="utf-8") as f:
    tasks = json.load(f)

# Replace any prior install task (idempotent if the generator is re-run).
tasks = [t for t in tasks if not t.get("label", "").startswith("xmake: install project tasks")]
tasks.append(install_task)

with open("languages/xmake/tasks.json", "w", encoding="utf-8") as f:
    json.dump(tasks, f, indent=2, ensure_ascii=False)
    f.write("\n")

print("Wrote languages/xmake/tasks.json with", len(tasks), "tasks")
