[English](README.md) | [简体中文](README.zh-CN.md)

# OpenTyless Rust CLI

OpenTyless 是一个面向桌面工作流的轻量 Rust 语音转写 CLI。它不依赖程序自己监听全局热键，而是让桌面环境触发命令，再由用户级 daemon 完成录音、转写和整理。

核心设计：

- 用桌面快捷键触发 CLI，而不是全局键盘 hook
- 后台常驻使用 `systemd --user`
- 守护进程通过 Unix socket 接收 `start-record` / `stop-record` / `toggle-record`
- 处理链路是 `ASR -> Qwen Flash 整理 -> Markdown 输出`

发布和打包流程见 [发布指南](docs/RELEASE.zh-CN.md)。

## 发布方式

当前仓库采用一套主功能、一套发布逻辑：

- 核心 Rust 代码只有一套
- release 产物只有一条主发布线
- Wayland / X11 的差异通过运行时环境检测和安装默认值处理

## 为什么这样设计

在现代 Linux 桌面里，尤其是 GNOME Wayland，后台程序并不适合可靠监听全局快捷键；而 X11 与 Wayland 在剪贴板、托盘和桌面集成上又存在不同默认值。

因此项目结构是：

- `opentyless-rs daemon`：后台常驻
- `opentyless-rs start-record`：通知 daemon 开始录音
- `opentyless-rs stop-record`：通知 daemon 停止录音并处理
- `opentyless-rs toggle-record`：推荐绑定到快捷键

快捷键由桌面环境负责，录音和处理由 CLI 负责。

## 功能清单

- daemon 模式和本地 Unix socket
- `once --seconds N` 一次性录音处理
- `systemd --user` 服务安装
- GNOME 快捷键安装
- GNOME / AppIndicator 托盘菜单
- 转写完成后自动复制剪贴板
- 开始录音、停止转写、处理完成的桌面通知

## 安装

当前支持三类安装/分发方式：

- `git clone` + `./install.sh`
  这是源码安装；会在用户自己的机器上本地编译 Rust 二进制。
- GitHub Release 压缩包
  这是预编译二进制安装；用户直接下载你提前构建好的产物再安装。
- `npm i -g opentyless@latest`
  这是包管理器分发；npm 包本身是一个很薄的安装壳，实际优先使用随包附带的预编译 Rust 二进制，缺失时再回退到 GitHub Release。

如果你是开发者，推荐使用源码安装；如果你是普通用户，推荐使用 GitHub Release 或 npm。

### 方案零：通过 npm 安装 Rust 二进制分发包

适合你希望像 `codex` 一样，用 `npm` 统一安装和升级命令行工具。

```bash
npm i -g opentyless@latest
```

安装完成后可直接执行：

```bash
opentyless --help
opentyless doctor
```

这个 npm 包本身不重新实现核心逻辑，而是优先携带预编译 Rust 二进制；安装时由 `postinstall` 解包到运行时目录，再由一个很薄的 Node 启动器转发命令。若随包运行时缺失，才回退到 GitHub Release 下载。

更多说明见：[`docs/NPM.zh-CN.md`](docs/NPM.zh-CN.md)。

### 方案一：一键安装

```bash
git clone <your-repo-url>
cd opentyless
chmod +x install.sh
./install.sh
```

安装脚本会：

- 安装 Debian / Ubuntu 常见依赖
- 如果本机没有 Rust，则安装 Rust
- 编译 release 二进制
- 安装 `opentyless-rs` 到 `~/.local/bin`
- 安装热键包装脚本 `opentyless-hotkey-toggle`
- 如有需要，从 `.env.example` 自动生成 `.env`
- 安装并可启用 `systemd --user` 服务
- 安装 GNOME 托盘自启动

常用参数：

```bash
./install.sh --skip-deps
./install.sh --no-service
./install.sh --no-tray
./install.sh --install-shortcut --binding 'F8'
```

在 Omarchy / Hyprland 下推荐：

```bash
./install.sh --install-shortcut
./install.sh --install-shortcut --binding 'SUPER + V' --binding 'CTRL + V'
```

Hyprland 默认同时绑定 `SUPER + V` 和 `CTRL + V`，都指向 `toggle-record`。
`CTRL + V` 会被合成器拦截，应用内粘贴在该绑定存在期间不可用。若要改成别的键，请自己传 `--binding`。

安装器会先备份 `~/.config/hypr/bindings.lua`，然后维护一个带标记、可重复更新的
OpenTyless 配置区块。若快捷键已被占用，安装会停止；确认覆盖时显式增加
`--force`。运行 `./install.sh --uninstall-shortcut` 可移除托管区块。

systemd 用户服务会把 `WorkingDirectory` 和 `EnvironmentFile` 固定到
`~/.config/opentyless`，并在 `install-service` 时把 `.env` 复制过去。
安装完成后即使源码目录被移动或删除，登录自启动也不会因此失败。

### 方案二：手动安装

Debian / Ubuntu 依赖：

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config curl git ffmpeg xclip wl-clipboard libasound2-dev libdbus-1-dev
```

安装 Rust：

```bash
curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

构建：

```bash
cargo build --release
```

安装二进制：

```bash
mkdir -p ~/.local/bin
cp ./target/release/opentyless-rs ~/.local/bin/opentyless-rs
chmod +x ~/.local/bin/opentyless-rs
```

如果 `~/.local/bin` 不在 `PATH` 里，加入：

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## 配置

先创建环境文件：

```bash
cp .env.example .env
```

最少需要：

```env
DASHSCOPE_API_KEY=your_dashscope_key
```

默认已经按 DashScope compatible mode 预设：

- Base URL：`https://dashscope.aliyuncs.com/compatible-mode/v1`
- ASR 模型：`qwen3-asr-flash`
- 整理模型：`qwen-flash`

常用可选变量：

- `RECORDER_CMD`：自定义录音命令
- `FLASH_PROMPT`：覆盖整理阶段的 system prompt；程序默认已经把原文包进 `<raw_transcript>...</raw_transcript>`，并强制 JSON mode + 低温度解码
- `FLASH_EXTRA_BODY_JSON`：向整理请求体 merge 额外 JSON；默认已包含 `response_format={"type":"json_object"}`、`temperature=0.1`、`seed=7`
- `AUTO_COPY_TO_CLIPBOARD`：是否自动复制
- `CLIPBOARD_COMMAND`：显式指定 `xclip`、`wl-copy` 或自定义命令
- `ENABLE_NOTIFICATIONS`：是否启用桌面通知
- `OUTPUT_DIR`、`RUN_DIR`、`STATE_DIR`、`SOCKET_PATH`：覆盖运行时路径

默认运行目录是绝对 XDG 路径：

- `RUN_DIR`：`~/.local/state/opentyless/run`
- `STATE_DIR`：`~/.local/state/opentyless/state`
- `OUTPUT_DIR`：`~/.local/share/opentyless/outputs`
- `SOCKET_PATH`：`~/.local/state/opentyless/run/opentyless.sock`

## 安装方式说明

### 为什么这里同时保留 `install.sh`、Release 和 npm

- `install.sh`
  适合源码安装和开发调试；它会在本地执行 `cargo build --release`，因此依赖用户机器具备 Rust 工具链。
- GitHub Release
  适合给普通用户提供“开箱即用”的压缩包；不要求用户本地安装 Rust。
- npm
  适合提供类似现代 CLI 工具的安装与升级体验；本质上仍然是在分发预编译二进制，只是入口换成了 npm。

### 为什么不只用 Cargo

- `cargo` 非常适合 Rust 开发者
- 但 `cargo install` 的主流使用方式通常仍然是下载源码并在本地编译
- 对普通 Linux 桌面用户来说，GitHub Release 或 npm 的体验通常更简单

因此当前项目的定位是：

- 开发和调试：`cargo`
- 源码安装：`git clone` + `./install.sh`
- 成品分发：GitHub Release / npm

## 快速开始

检查环境：

```bash
opentyless-rs doctor
```

启动 daemon：

```bash
opentyless-rs daemon
```

开始 / 停止录音：

```bash
opentyless-rs start-record
opentyless-rs stop-record
```

或者直接切换：

```bash
opentyless-rs toggle-record
```

一次性测试整条链路：

```bash
opentyless-rs once --seconds 5
```

查看状态：

```bash
opentyless-rs status
```

## CLI Reference

### 录音与转写

- `opentyless-rs daemon`
  启动后台守护进程，负责录音生命周期。
- `opentyless-rs start-record`
  通知 daemon 开始录音。
- `opentyless-rs stop-record`
  通知 daemon 停止录音并立即返回，然后在后台开始转写和整理；现在会先提示“已停止录音，正在转写”，完成后再提示“转写完成”。
- `opentyless-rs toggle-record`
  在开始录音和停止并转写之间切换；最适合绑快捷键。
  转写期间再次触发不会开始新录音，只会提示“正在转写”。
- `opentyless-rs once --seconds 5`
  不走 daemon，直接录音固定秒数并处理一次。

### 状态与排障

- `opentyless-rs status`
  输出 daemon 是否运行、socket 路径，以及当前状态 JSON。
- `opentyless-rs doctor`
  输出实际录音命令、API 配置状态、剪贴板命令、通知开关和运行目录。
- `opentyless-rs logs -n 50`
  查看 `systemd --user` 服务最近日志。
- `opentyless-rs copy-last`
  把最近一次整理后的文本重新复制到剪贴板。

### 服务管理

- `opentyless-rs install-service --enable`
  安装用户服务；加 `--enable` 时会立刻启用并启动。
- `opentyless-rs uninstall-service`
  停止并删除用户服务。
- `opentyless-rs service-status`
  查看 `opentyless.service` 的 systemd 状态。
- `opentyless-rs start`
  等价于 `systemctl --user start opentyless.service`。
- `opentyless-rs stop`
  等价于 `systemctl --user stop opentyless.service`。
- `opentyless-rs restart`
  等价于 `systemctl --user restart opentyless.service`。

### 桌面集成

- `opentyless-rs install-gnome-shortcut --binding '<Super>space'`
  通过 `gsettings` 创建 GNOME 自定义快捷键。
- `opentyless-rs install-tray-autostart`
  安装 GNOME 登录自启动项。
- `opentyless-rs tray`
  启动系统托盘菜单。

### 帮助

- `opentyless-rs --help`
  查看总帮助。
- `opentyless-rs <command> --help`
  查看某个子命令的帮助和参数。

## 自启动与托盘

安装并启用用户服务：

```bash
opentyless-rs install-service --enable
```

查看服务状态：

```bash
opentyless-rs service-status
opentyless-rs logs -n 100
```

安装托盘自启动：

```bash
opentyless-rs install-tray-autostart
```

手动启动托盘：

```bash
opentyless-rs tray
```

## Wayland 与 X11 说明

- Wayland 下推荐把桌面快捷键绑定到 `toggle-record`，程序不依赖自己监听全局按键
- Omarchy / Hyprland 下可使用安装器的 `--install-shortcut` 管理 Lua 快捷键配置；默认同时使用 `SUPER + V` 和 `CTRL + V`
- X11 与 Wayland 共用同一套 daemon、service、tray 和转写主流程
- 当前程序会优先根据 `XDG_SESSION_TYPE` 选择剪贴板默认值：
  - `wayland` 优先 `wl-copy`
  - `x11` 优先 `xclip`
- 当前默认策略不是向焦点输入框直接注入文本，而是复制到剪贴板后由你粘贴
- 如果系统同时装了多套剪贴板工具，也可以手动设置 `CLIPBOARD_COMMAND`

## 常见问题

### 缺少 API Key

如果你看到缺少 `ASR_API_KEY` 或 `FLASH_API_KEY`，确认 `.env` 里至少有：

```env
DASHSCOPE_API_KEY=your_dashscope_key
```

### 无法连接 daemon

重新安装或重启服务：

```bash
opentyless-rs install-service --enable
opentyless-rs restart
opentyless-rs status
```

### 找不到录音命令

安装 `ffmpeg`，或者设置自定义录音命令：

```env
RECORDER_CMD=ffmpeg -hide_banner -loglevel error -f pulse -i default -ac 1 -ar 16000
```

### 托盘显示了，但点击没反应

重启服务和托盘：

```bash
systemctl --user restart opentyless.service
pkill -f 'opentyless-rs tray' || true
setsid -f ~/.local/bin/opentyless-rs tray >/tmp/opentyless-tray.log 2>&1
```

## Roadmap

- 提升跨桌面环境的热键稳定性，减少对手工调试 GNOME / Wayland / X11 快捷键的依赖
- 评估更可靠的热键触发方案，例如桌面扩展、平台专用桥接层或更清晰的快捷键诊断工具
- 支持多语言界面与文档
- 支持多种 ASR / LLM API 提供商与可切换配置
