你是一个资深 Rust / Tauri / Windows 桌面应用工程师。请帮我从零开始实现一个 Windows-only 托盘程序，产品名为 DevDock。

DevDock 的定位：

DevDock 是一个运行在 Windows 上的本地开发服务管理托盘程序。它通过 JSON 配置文件读取服务列表，在系统托盘右键菜单中展示服务分组、服务状态和可用操作。它可以启动、终止、重启普通进程类服务，也可以为特定类型服务提供额外操作。例如 React Native Metro 服务可以通过向 stdin 写入字符 `r` 来 reload app，通过写入 `d` 打开 dev menu。DevDock 不需要把已运行服务附加到 Windows Terminal；Windows Terminal 只用于 tail 日志文件。

技术栈要求：

使用 Rust + Tauri 实现。项目面向 Windows，不需要兼容 macOS 或 Linux。优先使用 Tauri v2。前端 UI 可以非常简单，核心功能放在 Rust 后端。需要实现系统托盘图标和右键菜单。若 Tauri v2 托盘 API 可用，则使用 Tauri 官方能力；如果需要额外 crate，请选择维护活跃、Windows 支持好的方案。

不要使用 Electron。不要使用 .NET。不要把普通开发进程实现成 Windows Service。不要实现内置 PTY / xterm.js；当前版本采用 supervisor + 日志 tail 方案。

第一阶段目标是 MVP，可运行、结构清晰、可扩展。

核心需求：

1. 应用启动后常驻系统托盘。

2. 从 JSON 配置文件读取服务列表。

3. 配置文件默认路径为：

   `%APPDATA%/DevDock/devdock.config.json`

   如果文件不存在，则自动创建一个示例配置文件。

4. 托盘右键菜单按 group 分组展示服务。

5. 每个服务菜单项显示当前状态，例如：

   `React Native Metro - Stopped`
   `React Native Metro - Running`
   `React Native Metro - Running · Ready`
   `API Server - Failed`

6. 普通服务支持默认操作：

   服务 stopped 时显示：

   * Start
   * Open Log，如果配置了 log.file
   * Open Log in Terminal，如果配置了 log.file

   服务 running 时显示：

   * Stop
   * Restart
   * Open Log
   * Open Log in Terminal
   * 配置中的 when=running 或 when=any 的 actions

   服务 failed 时显示：

   * Start
   * Restart
   * Open Log
   * Open Log in Terminal
   * 配置中的 when=failed 或 when=any 的 actions

7. 支持三类服务：

   * `process`
   * `react-native`
   * `windows-service`

8. `process` 服务：

   由 DevDock 使用 Rust 启动外部命令。需要支持：

   * cwd
   * command
   * args
   * env
   * stdout/stderr 写入日志文件
   * stdin 可选保留
   * stop
   * restart
   * kill process tree

   第一版如果进程树终止实现较复杂，可以先封装接口，并在 Windows 下使用 `taskkill /PID <pid> /T /F` 实现。

9. `react-native` 服务：

   本质上继承 `process` 行为，但默认应保留 stdin。
   需要支持配置中的 stdin actions，例如：

   {
   "id": "reload",
   "label": "Reload app",
   "when": "running",
   "kind": "stdin",
   "input": "r"
   }

   触发该 action 时，向该服务进程 stdin 写入配置中的 input 字符。

   需要支持通过 stdout/stderr 解析状态。配置中可以定义 status.patterns，例如 ready、building、error。只要输出行匹配对应字符串，就更新服务的 detail status。

10. `windows-service` 服务：

第一版可以先设计结构和菜单，但如果 Windows Service 控制实现复杂，可以先提供明确的模块边界和 TODO。
最好实现基础功能：

* 查询服务状态
* start
* stop
* restart

可以调用 Windows 命令行工具 `sc.exe` 或 PowerShell，也可以使用 Rust Windows API crate。优先选择实现简单且可靠的方式。普通开发进程的 supervisor 功能优先级更高。

11. 日志：

每个 process/react-native 服务启动后，将 stdout 和 stderr 追加写入配置中的 log.file。
如果未配置 log.file，则自动生成：

`%APPDATA%/DevDock/logs/<service-id>.log`

Open Log：用系统默认程序打开日志文件。
Open Log in Terminal：用 Windows Terminal tail 日志文件。默认命令模板为：

`wt.exe new-tab --title "<service-name>" powershell -NoExit -Command "Get-Content -Path '<log-file>' -Wait"`

路径中有空格时必须正确转义。

12. 配置文件格式使用 JSON。

请实现并使用下面的配置结构。字段使用 camelCase。

{
"version": 1,
"app": {
"name": "DevDock",
"logDir": "${APP_DATA}/DevDock/logs",
"terminal": {
"program": "wt.exe",
"shell": "powershell",
"tailCommand": "Get-Content -Path "{logFile}" -Wait"
}
},
"groups": [
{
"id": "mobile",
"name": "Mobile"
},
{
"id": "backend",
"name": "Backend"
}
],
"services": [
{
"id": "rn-mobile",
"name": "React Native Metro",
"type": "react-native",
"group": "mobile",
"cwd": "D:/workspace/mobile-app",
"command": "npx",
"args": ["react-native", "start"],
"env": {
"NODE_ENV": "development"
},
"process": {
"killTree": true,
"restartDelayMs": 1000,
"startOnLaunch": false
},
"log": {
"file": "${APP_DATA}/DevDock/logs/rn-mobile.log",
"rotate": true,
"maxSizeMb": 20,
"maxFiles": 5
},
"status": {
"mode": "output-pattern",
"patterns": {
"starting": [
"Welcome to Metro",
"Starting Metro"
],
"ready": [
"Metro waiting on",
"Dev server ready"
],
"building": [
"Building",
"Bundling"
],
"error": [
"error:",
"Failed to"
]
}
},
"actions": [
{
"id": "restart-reset-cache",
"label": "Restart with cache reset",
"when": "any",
"kind": "restart",
"command": "npx",
"args": ["react-native", "start", "--reset-cache"]
}
]
},
{
"id": "api-server",
"name": "API Server",
"type": "process",
"group": "backend",
"cwd": "D:/workspace/api",
"command": "npm",
"args": ["run", "dev"],
"env": {
"NODE_ENV": "development"
},
"process": {
"killTree": true,
"restartDelayMs": 1000,
"startOnLaunch": false
},
"log": {
"file": "${APP_DATA}/DevDock/logs/api-server.log",
"rotate": true,
"maxSizeMb": 20,
"maxFiles": 5
},
"actions": []
},
{
"id": "redis",
"name": "Redis Service",
"type": "windows-service",
"group": "backend",
"windowsService": {
"serviceName": "Redis"
},
"log": {
"file": "D:/tools/redis/redis.log"
},
"actions": []
}
]
}

需要实现的 Rust 数据结构：

* DevDockConfig
* AppConfig
* TerminalConfig
* GroupConfig
* ServiceConfig
* ServiceType
* ProcessOptions
* WindowsServiceOptions
* LogOptions
* StatusOptions
* ActionConfig
* ActionKind
* ActionWhen

要求 serde 支持 JSON 反序列化，字段使用 camelCase。

服务状态模型：

定义统一状态：

* stopped
* starting
* running
* stopping
* restarting
* failed
* unknown

同时支持可选 detail status，例如：

* ready
* building
* error

可以定义：

struct RuntimeServiceState {
lifecycle: ServiceLifecycle,
detail: Option<String>,
pid: Option<u32>,
lastError: Option<String>
}

内部模块建议：

src-tauri/src/

* main.rs
* config.rs
* paths.rs
* tray.rs
* service.rs
* process_manager.rs
* windows_service.rs
* logs.rs
* terminal.rs

模块职责：

config.rs：
读取、创建、校验 devdock.config.json。
支持变量展开：

* `${APP_DATA}` 映射到 Windows AppData Roaming 目录
* `${CONFIG_DIR}` 映射到 DevDock 配置目录
* `${LOG_DIR}` 映射到 app.logDir

paths.rs：
提供 app data、config path、log dir 等路径函数。

service.rs：
定义服务状态、服务运行时注册表、统一操作接口。
提供 start_service、stop_service、restart_service、run_action、get_service_state 等函数。

process_manager.rs：
管理 process/react-native 服务。
要求：

* 使用 tokio::process::Command 或 std::process::Command
* stdout/stderr 异步读取并写入日志
* 按行做 status pattern 匹配
* 保存 child handle、stdin handle、pid
* stop 时如果 killTree=true，调用 taskkill /PID <pid> /T /F
* restart 时先 stop，等待 restartDelayMs，再 start
* 进程退出时更新状态为 stopped 或 failed

windows_service.rs：
封装 windows-service 类型操作。
第一版可使用 sc.exe query/start/stop，注意解析状态。
如果实现复杂，保留清晰 TODO，但不能影响 process/react-native 功能。

logs.rs：
创建日志目录。
打开日志文件。
追加写日志。
可以先不实现 rotate，但保留配置字段。

terminal.rs：
打开 Windows Terminal tail 日志。
实现路径和命令参数转义。

tray.rs：
创建托盘图标和菜单。
根据当前配置和状态动态构建菜单。
点击菜单项后调用 service.rs 中的操作。
服务状态变化后刷新托盘菜单。
菜单中需要有：

* Reload Config
* Open Config File
* Open Logs Directory
* Quit

如果 Tauri 的 tray menu 动态刷新存在限制，先实现可用版本：每次操作完成后重建菜单或更新菜单文本。

前端窗口：

MVP 可以没有主窗口，或者只有一个简单窗口显示 DevDock 正在运行。
重点是托盘功能。
如果没有窗口，也要确保应用不因窗口关闭而退出。

错误处理要求：

使用 anyhow 或 thiserror。
所有用户可见错误要写入日志。
配置错误需要给出明确错误信息，例如：

services[0].command is required for type process
services[2].windowsService.serviceName is required for type windows-service
services[0].actions[0].kind=stdin requires process stdin support

开发约束：

1. 优先实现能运行的 MVP。
2. 不要过度抽象。
3. 不要实现数据库。
4. 不要实现远程服务。
5. 不要实现 Web API。
6. 不要实现登录/用户系统。
7. 不要实现插件系统。
8. 不要实现内置终端。
9. 不要引入复杂状态管理框架。
10. 代码结构要为后续扩展服务类型保留空间。

验收标准：

1. 启动 DevDock 后，系统托盘出现图标。
2. 如果配置文件不存在，自动创建示例配置。
3. 右键托盘图标能看到配置里的服务列表。
4. process 服务可以启动。
5. process 服务启动后 stdout/stderr 写入日志文件。
6. running 状态下可以 Stop 和 Restart。
7. react-native 服务可以执行 stdin action，例如写入 `r`。
8. 可以打开日志文件。
9. 可以用 Windows Terminal tail 日志。
10. Reload Config 可以重新读取配置并刷新菜单。
11. Quit 可以正常退出应用，并尝试停止由 DevDock 启动的子进程。
12. 配置错误时不要 panic，要给出可诊断错误。

请先生成项目结构、核心 Rust 类型、配置加载、进程管理、日志、终端打开和托盘菜单的 MVP 实现。实现完成后，请给出运行方式、配置文件位置、已完成能力和未完成 TODO。
