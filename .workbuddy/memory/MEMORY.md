# xmake-zed 项目长期记忆

## 项目概况
Zed 编辑器的 xmake 扩展 (fork: YangZhenxun/xmake-zed, 上游: xmake-io/xmake-zed)。
Rust 编译为 wasm32-wasip2，产物 extension.wasm。依赖 zed_extension_api 0.7.0。

## 上游 Issues 状态 (截至 2026-06-29)
- #1 Add a debug task → 已修复(本 fork commit 4e3f54c)，实现完整 debug locator + adapter。
- #3 compile_commands.json 输出到 .zed → 已修复(任务改为输出到 .zed/)。
- #4 任务只在打开 xmake.lua 时可用 → 已修复(install project tasks 任务写 .zed/tasks.json)。

## Zed 扩展开发关键约束 (踩过的坑)
- WASM 沙箱: 用 `zed::Command`(WIT process API) 跑外部命令，**别用** std::process::Command。
- `zed::Command` 无 cwd 字段；要换目录在脚本里 os.cd。
- 别用 std::env::current_exe() 定位 assets；用 include_str! 嵌入。
- tasks.json: command+args 是 argv 不走 shell，串联用 sh -c；$argv 不存在用 ${ZED_SELECTED_TEXT:default}。
- 扩展无法注册项目级全局任务；只能 languages/<lang>/tasks.json(语言绑定)。
- DAP locator 两阶段都必须实现: create_scenario + run_dap_locator。

## 构建
`cargo build --release --target wasm32-wasip2` → cp 产物到 extension.wasm。
cargo 末尾写 ~/.cargo 缓存会被沙箱拦(不影响编译)。
