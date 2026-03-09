# OpenTyless Rust CLI

一个面向 **Wayland** 的轻量 Rust CLI：

- 不自己监听全局按键
- 由 **桌面环境快捷键** 触发 CLI 命令
- 后台常驻使用 `systemd --user`
- 守护进程通过 **Unix socket** 接收 `start-record` / `stop-record` / `toggle-record`
- 录音后执行：`ASR -> Qwen Flash 整理 -> 输出 Markdown`

## 为什么这样设计

在 Wayland 下，普通后台程序通常**不能可靠监听全局热键**。

所以这里改成更稳的架构：

- `opentyless-rs daemon`：后台常驻
- `opentyless-rs start-record`：开始录音
- `opentyless-rs stop-record`：停止录音并处理
- `opentyless-rs toggle-record`：切换录音状态

然后把系统快捷键绑定到这些 CLI 命令即可。

## 功能清单

- `daemon`：启动守护进程并监听 Unix socket
- `start-record`：通知守护进程开始录音
- `stop-record`：通知守护进程停止录音并执行 ASR + Flash
- `toggle-record`：开始/停止录音切换
- `status`：查看本地状态文件
- `once --seconds N`：不走 daemon，直接录音 N 秒并处理一次
- `doctor`：检查录音命令、环境变量、目录与 socket 配置
- `install-service --enable`：安装并启用 systemd 用户服务
- `install-gnome-shortcut`：通过 CLI 安装 Ubuntu GNOME 自定义快捷键
- `install-tray-autostart`：安装 GNOME 登录后自动启动托盘
- `start` / `stop` / `restart`：控制后台服务
- `service-status`：查看 systemd 服务状态
- `logs -n 50`：查看最近日志
- `copy-last`：把最近一次整理结果复制到剪贴板
- `tray`：启动 GNOME 托盘菜单
- `uninstall-service`：卸载用户服务

## CLI Reference

### 录音与转写

- `opentyless-rs daemon`
  启动后台守护进程；通常配合 `systemd --user` 常驻运行。
- `opentyless-rs start-record`
  通知 daemon 开始录音；成功时会更新本地状态并发送“开始录音”通知。
- `opentyless-rs stop-record`
  通知 daemon 停止录音，然后执行转写和整理；现在会先提示“已停止录音，正在转写”，完成后再提示“转写完成”。
- `opentyless-rs toggle-record`
  在“开始录音”和“停止并转写”之间切换；最适合绑定成桌面快捷键。
- `opentyless-rs once --seconds 5`
  不经过 daemon，直接录音指定秒数并处理一次；适合快速测试整条链路。

### 状态与排障

- `opentyless-rs status`
  输出 daemon 是否在运行、socket 路径，以及当前状态 JSON。
- `opentyless-rs doctor`
  检查录音命令、API 环境变量、目录、socket、剪贴板命令和通知开关。
- `opentyless-rs logs -n 50`
  查看 `systemd --user` 服务最近日志；`-n` 默认是 `50`。
- `opentyless-rs copy-last`
  把最近一次整理后的文本重新复制到剪贴板。

### 服务管理

- `opentyless-rs install-service --enable`
  安装用户级 systemd 服务；加 `--enable` 时会立刻启用并启动。
- `opentyless-rs uninstall-service`
  停止并删除用户服务文件。
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
  通过 `gsettings` 自动创建 GNOME 自定义快捷键，默认绑定 `<Super>space`。
- `opentyless-rs install-tray-autostart`
  安装 GNOME 登录自启动项，让托盘随桌面会话一起启动。
- `opentyless-rs tray`
  启动系统托盘菜单；支持开始/停止录音、复制最近结果、打开输出目录、显示状态通知。

### 帮助

- `opentyless-rs --help`
  查看总帮助。
- `opentyless-rs <command> --help`
  查看某个子命令的帮助和参数。

## 项目文件

- `src/main.rs`：主程序
- `Cargo.toml`：Rust 依赖配置
- `.env.example`：环境变量模板
- `install.sh`：新机器一键安装脚本

## 安装

### 方案一：一键安装，适合新电脑

适用场景：

- 新机器还没装 Rust
- 想自动装依赖、编译、安装 service、安装托盘
- 想顺手安装一个更方便排查问题的热键包装脚本

执行：

```bash
git clone <your-repo-url>
cd opentyless
chmod +x install.sh
./install.sh
```

脚本会做这些事：

- 安装 Rust（如果本机没有）
- 在 Debian/Ubuntu 上安装常用系统依赖
- 编译 release 版本
- 安装二进制到 `~/.local/bin/opentyless-rs`
- 安装热键包装脚本到 `~/.local/bin/opentyless-hotkey-toggle`
- 如果没有 `.env`，自动从 `.env.example` 复制
- 安装并启用 `systemd --user` 服务
- 安装 GNOME 托盘自启动
- 最后执行一次 `doctor`

常用参数：

```bash
./install.sh --skip-deps
./install.sh --no-service
./install.sh --no-tray
./install.sh --install-shortcut --binding 'F8'
```

### 方案二：手动安装

适合你想自己控制每一步。

#### 1) 安装系统依赖

Ubuntu / Debian：

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config curl git ffmpeg xclip wl-clipboard libasound2-dev libdbus-1-dev
```

#### 2) 安装 Rust

```bash
curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

#### 3) 拉代码并构建

```bash
git clone <your-repo-url>
cd opentyless
cargo build --release
```

#### 4) 安装二进制到本地 PATH

```bash
mkdir -p ~/.local/bin
cp ./target/release/opentyless-rs ~/.local/bin/opentyless-rs
chmod +x ~/.local/bin/opentyless-rs
```

如果 `~/.local/bin` 不在 `PATH` 里，把下面这一行加到 `~/.bashrc` 或 `~/.zshrc`：

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## 构建

### 开发构建

```bash
cargo build
```

### 发布构建

```bash
cargo build --release
```

二进制默认路径：

- 开发版：`target/debug/opentyless-rs`
- 发布版：`target/release/opentyless-rs`

## 配置

先复制环境变量模板：

```bash
cp .env.example .env
```

然后填写：

```env
DASHSCOPE_BASE_URL=https://dashscope.aliyuncs.com/compatible-mode/v1
DASHSCOPE_API_KEY=your_dashscope_key
ASR_MODEL=qwen3-asr-flash

FLASH_MODEL=qwen-flash
```

至少要填这个：

```env
DASHSCOPE_API_KEY=your_dashscope_key
```

你写在 `apikey.md` 里的信息已经体现在默认配置里：

- Base URL：`https://dashscope.aliyuncs.com/compatible-mode/v1`
- ASR 模型：`qwen3-asr-flash`
- 文本整理模型：`qwen-flash`

建议把真实密钥只放进 `.env`，不要继续保存在 `apikey.md` 这种会被提交的文件里。

### 可选配置

- `RECORDER_CMD`：自定义录音命令
- `ASR_EXTRA_FORM_JSON`：ASR 附加请求体 JSON
- `FLASH_EXTRA_BODY_JSON`：Flash 附加请求体 JSON
- `FLASH_PROMPT`：整理提示词
- `AUTO_COPY_TO_CLIPBOARD`：转写完成后自动复制到剪贴板
- `CLIPBOARD_COMMAND`：自定义剪贴板命令
- `ENABLE_NOTIFICATIONS`：是否显示 GNOME 桌面通知
- `OUTPUT_DIR`：结果输出目录
- `RUN_DIR`：运行时目录
- `STATE_DIR`：状态目录
- `SOCKET_PATH`：daemon Unix socket 路径

默认整理 prompt 已固定为“校对员模式”：

- 保留原意
- 问题保持为问题
- 明显口吃、口头禅和无意义重复可轻度合并
- 不回答，不扩写，不总结，不猜测

如果你启用 `--install-shortcut`，CLI 会优先把 GNOME 快捷键绑定到：

- `~/.local/bin/opentyless-hotkey-toggle`

这个包装脚本会把每次触发记录到：

- `~/.local/state/opentyless/hotkey.log`

这样一旦“按键没反应”，可以立刻区分是：

- GNOME 没派发按键
- 还是命令执行时出错

默认情况下这些目录会落到 XDG 绝对路径，避免托盘自启动和 systemd 服务因工作目录不同而互相找不到：

- `RUN_DIR`: `~/.local/state/opentyless/run`
- `STATE_DIR`: `~/.local/state/opentyless/state`
- `OUTPUT_DIR`: `~/.local/share/opentyless/outputs`
- `SOCKET_PATH`: `~/.local/state/opentyless/run/opentyless.sock`

## 快速开始

### 1) 检查环境

```bash
cargo run -- doctor
```

### 2) 启动守护进程

```bash
cargo run -- daemon
```

### 3) 触发录音

```bash
cargo run -- start-record
cargo run -- stop-record
```

或者：

```bash
cargo run -- toggle-record
```

### 4) 查看状态

```bash
cargo run -- status
```

### 5) 直接测试一次

```bash
cargo run -- once --seconds 5
```

如果你已经安装到 `~/.local/bin/opentyless-rs`，也可以直接：

```bash
opentyless-rs doctor
opentyless-rs status
```

## 开机自启与后台常驻

### 安装并启用

```bash
cargo run -- install-service --enable
```

或者如果你已经装到了 PATH：

```bash
opentyless-rs install-service --enable
```

它会自动生成：

- `~/.config/systemd/user/opentyless.service`
- 并执行 `daemon-reload`
- 如果传了 `--enable`，还会执行 `enable --now`

### 服务控制

```bash
cargo run -- start
cargo run -- stop
cargo run -- restart
```

### 查看服务状态与日志

```bash
cargo run -- service-status
cargo run -- logs -n 100
cargo run -- status
cargo run -- copy-last
cargo run -- tray
```

## GNOME 状态栏 / 托盘

Ubuntu GNOME 默认带 `ubuntu-appindicators` 扩展，所以本项目现在支持托盘入口。

### 手动启动托盘

```bash
./target/release/opentyless-rs tray
```

托盘菜单支持：

- 开始录音 / 停止录音
- 复制最近结果
- 打开输出目录
- 显示状态通知

### 登录自动启动托盘

```bash
./target/release/opentyless-rs install-tray-autostart
```

### 卸载

```bash
cargo run -- uninstall-service
```

## Wayland 下如何绑定快捷键

你现在是 **Ubuntu 定制的 GNOME 会话**，最稳的方式是直接装一个 **切换式快捷键**。

### 一条命令安装 GNOME 快捷键

```bash
./target/release/opentyless-rs install-gnome-shortcut --binding '<Super>space'
```

这条命令会通过 `gsettings` 自动创建一个自定义快捷键：

- 名称：`OpenTyless Toggle`
- 命令：`opentyless-rs toggle-record`
- 推荐按键：`Super + Space`

### 推荐方式

- 第一次按：开始录音
- 第二次按：停止录音并执行 ASR + Flash
- 整理后的文本会自动复制到剪贴板，随后你可以直接 `Ctrl + V`
- 同时会弹出 GNOME 通知，托盘 tooltip 会显示最近状态

如果你以后想拆成两个快捷键，也可以手动在 GNOME 设置里配：

- `opentyless-rs start-record`
- `opentyless-rs stop-record`

### 例如绑定到发布版二进制

```bash
/path/to/opentyless/target/release/opentyless-rs toggle-record
```

如果你已经启用了中文输入法，`Ctrl + Space` 很容易和输入法切换冲突；Ubuntu GNOME 推荐优先用 `<Super>space`，备选可以用 `<Ctrl><Alt>space` 或 `<Super>Return`。

## 关于“光标处输入”

在 **GNOME Wayland** 下，普通用户态程序通常不能稳定地向当前焦点输入框直接注入按键事件；这不是本项目单独能绕过的限制。

因此本项目当前采用的稳定方案是：

- 录音完成后自动把整理文本复制到剪贴板
- 你在目标输入框里直接 `Ctrl + V`

如果后续你愿意额外安装带更高权限的输入注入工具，我可以再给你扩展“自动粘贴到光标处”的实验性模式。

## 常见报错

### `缺少环境变量: ASR_API_KEY` 或 `FLASH_API_KEY`

原因：

- `.env` 没填 `DASHSCOPE_API_KEY`
- 或者你把 `ASR_API_KEY` / `FLASH_API_KEY` 留空，同时也没填 `DASHSCOPE_API_KEY`

处理：

```bash
sed -n '1,80p' .env
```

确认至少有：

```env
DASHSCOPE_API_KEY=your_dashscope_key
```

### `无法连接 daemon`

原因：

- 后台服务没启动
- 或者之前用了相对路径配置，导致 tray 和 daemon 不在同一个 socket 上

处理：

```bash
opentyless-rs install-service --enable
opentyless-rs restart
opentyless-rs status
```

确认 `socket_path` 是绝对路径，例如：

```text
/home/yourname/.local/state/opentyless/run/opentyless.sock
```

### `未找到录音命令`

原因：

- 没安装 `ffmpeg`
- 也没安装 `arecord`

处理：

```bash
sudo apt-get install -y ffmpeg
```

或者自定义：

```env
RECORDER_CMD=ffmpeg -hide_banner -loglevel error -f pulse -i default -ac 1 -ar 16000
```

### 托盘能显示，但菜单点了没反应

原因：

- 旧版本常见于相对路径导致 tray/daemon 不共用一个 socket
- 或者服务没有重启到新版本

处理：

```bash
systemctl --user restart opentyless.service
pkill -f 'opentyless-rs tray' || true
setsid -f ~/.local/bin/opentyless-rs tray >/tmp/opentyless-tray.log 2>&1
```

### `opentyless-rs: command not found`

原因：

- `~/.local/bin` 不在 `PATH`

处理：

```bash
export PATH="$HOME/.local/bin:$PATH"
```

并把它写入 shell 配置文件。

## DashScope 兼容模式请求格式

### ASR

- 默认请求地址：`DASHSCOPE_BASE_URL/chat/completions`
- Header：`Authorization: Bearer <DASHSCOPE_API_KEY>`
- JSON Body 核心结构：

```json
{
  "model": "qwen3-asr-flash",
  "messages": [
    {
      "role": "user",
      "content": [
        {
          "type": "input_audio",
          "input_audio": {
            "data": "<base64_audio>",
            "format": "wav"
          }
        }
      ]
    }
  ]
}
```

### Flash

- 默认请求地址：`DASHSCOPE_BASE_URL/chat/completions`
- Header：`Authorization: Bearer <DASHSCOPE_API_KEY>`
- JSON Body：

```json
{
  "model": "qwen-flash",
  "messages": [
    {
      "role": "system",
      "content": "整理提示词"
    },
    {
      "role": "user",
      "content": "ASR 输出文本"
    }
  ]
}
```

## 运行目录说明

- `run/`：PID、录音临时文件、socket
- `state/status.json`：最近状态快照
- `outputs/`：最终整理结果

## 推荐使用流程

### 首次部署

```bash
cp .env.example .env
cargo build --release
./target/release/opentyless-rs doctor
./target/release/opentyless-rs install-service --enable
./target/release/opentyless-rs install-gnome-shortcut --binding '<Super>space'
./target/release/opentyless-rs install-tray-autostart
```

### 自动化测试

```bash
cargo test
```

### 日常使用

- 登录后 systemd 自动启动 daemon
- 登录后托盘可自动启动
- 你只需要按桌面快捷键
- 快捷键执行 `start-record` / `stop-record` / `toggle-record`
- 每次结果会写到 `outputs/`
- 托盘菜单也可以直接开始/停止/复制最近结果
- 出问题就看 `status` 和 `logs`

## 注意事项

- Wayland 下不建议依赖程序自行监听全局键盘
- 默认录音优先 `ffmpeg`，其次 `arecord`
- 如果阿里云接口格式和当前默认格式不同，把准确文档给我，我可以继续精确对齐
