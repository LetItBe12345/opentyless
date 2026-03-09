[English](README.md) | [简体中文](README.zh-CN.md)

# OpenTyless Rust CLI

OpenTyless 是一个面向桌面工作流的轻量 Rust 语音转写 CLI。它不依赖程序自己监听全局热键，而是让桌面环境触发命令，再由用户级 daemon 完成录音、转写和整理。

核心设计：

- 用桌面快捷键触发 CLI，而不是全局键盘 hook
- 后台常驻使用 `systemd --user`
- 守护进程通过 Unix socket 接收 `start-record` / `stop-record` / `toggle-record`
- 处理链路是 `ASR -> Qwen Flash 整理 -> Markdown 输出`

发布和打包流程见 [发布指南](docs/RELEASE.zh-CN.md)。

## 发布线

当前仓库维护两条发布线：

- `master`：默认的 Wayland 取向版本
- `x11`：面向 X11 的版本，安装脚本默认值和桌面启动行为更偏向 X11/GNOME

需要发哪个版本，就从对应分支构建和发布。

## 为什么这样设计

在现代 Linux 桌面里，尤其是 GNOME Wayland，后台程序并不适合可靠监听全局快捷键。

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
- `AUTO_COPY_TO_CLIPBOARD`：是否自动复制
- `CLIPBOARD_COMMAND`：显式指定 `xclip`、`wl-copy` 或自定义命令
- `ENABLE_NOTIFICATIONS`：是否启用桌面通知
- `OUTPUT_DIR`、`RUN_DIR`、`STATE_DIR`、`SOCKET_PATH`：覆盖运行时路径

默认运行目录是绝对 XDG 路径：

- `RUN_DIR`：`~/.local/state/opentyless/run`
- `STATE_DIR`：`~/.local/state/opentyless/state`
- `OUTPUT_DIR`：`~/.local/share/opentyless/outputs`
- `SOCKET_PATH`：`~/.local/state/opentyless/run/opentyless.sock`

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

### Wayland

- 推荐把桌面快捷键绑定到 `toggle-record`
- 剪贴板默认通常应当使用 `wl-copy`
- 当前默认策略不是向焦点输入框直接注入文本，而是复制到剪贴板后由你粘贴

### X11

- `x11` 分支和 `x11` 发布线会把安装默认值更偏向 `xclip`
- 如有需要，建议显式设置 `RECORDER_CMD`
- GNOME on X11 仍然适用同样的 daemon、service、tray 和快捷键流程

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
