# macOS 原生适配

macOS 13+；支持 Apple Silicon / Intel（在对应架构机器上构建）。Rust 负责 daemon、Unix socket、ASR、文本整理和历史文件；Swift 负责 AppKit 状态栏、Carbon 快捷键、鼠标侧键和 AVAudioRecorder 麦克风。

## 安装

需要 Rust stable 和 Xcode Command Line Tools（`xcode-select --install`）。

```bash
./scripts/install-macos.sh
# 或在 macOS 上直接运行 ./install.sh
```

安装器会编译 release Rust 和 Swift，生成本地 ad-hoc 签名应用，安装到 `/Applications/OpenTyless.app`，并把 `~/.local/bin/opentyless`、`opentyless-rs` 指向应用中的 Rust 二进制。原有应用和独立 CLI 会备份；已有 `.env` 保留，不覆盖密钥。更新时必须先结束正在录音或转写的任务。

可用 `OPENTYLESS_APPLICATIONS_DIR="$HOME/Applications" ./scripts/install-macos.sh` 指定应用目录。脚本从真实可执行文件路径寻找原生组件，支持符号链接和路径中的空格。

仅编译、不安装或更改后台服务：

```bash
./scripts/build-macos.sh
```

## 配置与使用

编辑 `~/.config/opentyless/.env`，填写：

```dotenv
DASHSCOPE_API_KEY='你的密钥'
```

从状态栏选择“重启后台服务 / 重新加载配置”，或运行 `opentyless restart`。

- 默认全局热键 **Control + Option + Space**：按一次开始，再按一次停止并转写。
- 状态栏“全局快捷键”菜单可切换 **Control + Shift + Space** 、**F8** 或 **Option + Space**，选择保存在本机 UserDefaults，重启后保留。若快捷键被占用会显示注册错误，保留原快捷键。
- 右上角麦克风图标显示就绪、录音、转写和错误状态；菜单提供开始/停止、取消录音、查看/复制最近结果、输出目录、配置、日志和后台重启。开始、转写、完成和失败会出现不抢焦点的短提示；录音/转写时持续显示，完成后自动收起。
- 第一次录音时需要 macOS 麦克风授权。拒绝后可到“系统设置 → 隐私与安全性 → 麦克风”重新允许。
- 原生录音使用当前系统默认输入设备，输出 16 kHz、16-bit、单声道 WAV；不需要 ffmpeg。
- 识别完成后使用 `pbcopy` 自动复制，按 **Command + V** 或鼠标下侧键粘贴。
- 鼠标上侧键按一下开始录音，再按一下停止并转写，与键盘录音快捷键执行同一个操作，无需长按。下侧键向当前应用发送 **Command + V**，粘贴当前剪贴板内容。处理中的录音开关会忽略重复触发。
- 侧键功能默认开启，需要在“系统设置 → 隐私与安全性 → 辅助功能”中允许 **OpenTyless**。状态栏“鼠标侧键 → 授权辅助功能…”可打开权限页；授权后自动开始监听。只用键盘组合快捷键时无需该权限。
- 本地 ad-hoc 签名在重新编译后会变化。若系统开关已开启，应用仍显示“请授权辅助功能”，先在辅助功能列表选中 OpenTyless，用减号移除旧条目，再用加号重新添加 `/Applications/OpenTyless.app` 并确认开启。单纯重启或重新开关旧条目可能无效；应用显示“上侧键录音 / 下侧键粘贴”表示监听已启动。
- 默认上侧键为鼠标前进键（零起始编号 4），下侧键为后退键（编号 3）。若鼠标方向相反，在“鼠标侧键”菜单勾选“交换上下侧键”；也可关闭侧键功能以恢复原有前进/后退行为。这两个选项重启后保留，滚轮中键和其他鼠标按键不受影响。若鼠标驱动已将侧键改成键盘按键，需要先在驱动中恢复原生前进/后退按键。
- API Key 未填写时，状态栏操作会先提示配置，不启动麦克风。

音频将发送至你在 `.env` 指定的服务商。默认是阿里云百炼的 `qwen3-asr-flash` + `qwen-flash`。密钥只写入本机权限为 `600` 的 `.env`，不应提交到 Git。

`RECORDER_CMD` 非空会覆盖原生录音；从旧 Linux/ffmpeg 配置迁移时应将它清空。`OUTPUT_DIR`、`STATE_DIR`、XDG 路径等仍由 Rust 读取。macOS 用本地浮层提示对齐 Ubuntu 版的录音/转写通知，不需要额外通知权限。最近结果只在主动打开预览时展开，提示浮层不展示转写正文。

## 登录自启动与服务管理

安装器默认启用两个用户 LaunchAgent：

- `~/Library/LaunchAgents/local.opentyless.daemon.plist`：Rust 后台，`RunAtLoad` + `KeepAlive`。
- `~/Library/LaunchAgents/local.opentyless.tray.plist`：原生状态栏，`RunAtLoad`；退出状态栏后本次登录不自动重开。

状态栏“登录时自动启动”可关闭或开启下次登录自启动。关闭时保留当前会话；退出应用会停止后台服务。重新打开应用会恢复当前会话后台；不会自行重新开启已关闭的登录自启动。

已有 CLI 在 macOS 上自动使用 launchd：

```bash
opentyless install-service --enable
opentyless install-tray-service --enable
opentyless service-status
opentyless tray-service-status
opentyless restart
opentyless stop
opentyless start
opentyless logs -n 50
opentyless tray-logs -n 50
```

卸载登录自启动（会停止对应服务）：

```bash
opentyless uninstall-service
opentyless uninstall-tray-service
```

后台收到 SIGTERM / SIGINT 时会停止录音子进程、清理 socket 和 PID，避免退出后继续占用麦克风。重新登录自动启动需要在真实登录会话中验证；不要把 `launchctl bootstrap` 成功等同于已经测试过重启电脑。

## 验证

```bash
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
./scripts/build-macos.sh
bash ./scripts/test-macos-mouse.sh
opentyless doctor
opentyless check-config
# 取消正在进行的录音（不上传转写）
opentyless cancel-record
```

自动测试包括模拟 ASR/LLM 的完整 CLI 流程、SIGINT 正常收尾 WAV、SIGTERM 停止录音子进程、配置文件保护、LaunchAgent 路径和退出语义。原生状态栏、实际按键事件、麦克风授权和真实服务商转写还需本机交互测试。

手动验收：检查状态栏存在；从其他应用按热键开始/停止；切换热键并重新打开应用检查保留；授权辅助功能后上侧键单击开始、再单击停止，下侧键在输入框粘贴且无前进/后退副作用；按住侧键不重复触发；交换侧键并重启验证保留，关闭后恢复原生行为；开关登录自启动并重新登录；拒绝/允许麦克风检查提示；断网或无效密钥检查错误；录音中停止 daemon 检查麦克风指示消失。

## 本次本机验证（2026-10-13，Apple Silicon）

- Rust 单元/集成测试、严格 Clippy、release Rust + Swift 编译、应用签名校验。
- LaunchAgent 已加载并运行，配置与密钥未进入仓库。重新登录尚未实测。
- 合成中文语音通过真实阿里云 ASR → Flash 流程；原生麦克风约 3 秒测试也完成转写和 Markdown 输出。
- 用户已确认从其他应用使用默认全局快捷键可以开始/停止。
- Ubuntu 风格浮层为本地 AppKit 界面；不会调用系统通知授权，也不会把结果正文放入短提示。

### 中文剪贴板与后台日志

LaunchAgent 的环境可能没有 locale，macOS `pbcopy` 此时可能退出成功却丢弃中文。macOS 适配会为 `pbcopy` 显式设置 UTF-8，并等待实际退出；失败时记录错误且不显示“已复制”。Linux `wl-copy`/`xclip` 和自定义驻留剪贴板命令仍保持非阻塞。日志包含转写开始/结束、字符数和复制状态，不记录转写正文或 API Key。

## 辅助功能授权与卡顿排查

升级本地 ad-hoc 签名应用后，旧授权条目可能仍显示开启，但新进程无法建立侧键监听。应先核对应用中的权限状态；必要时移除旧授权条目并重新添加当前应用。关闭侧键并重启状态栏可用于隔离输入卡顿；恢复授权和监听后还需实际验证粘贴、鼠标移动及打字。一次本地排查中用户确认恢复后不再卡顿，但尚不能将所有输入卡顿都归因于权限或系统重复绑定。侧键只执行录音切换和粘贴，不包含长按发送或自动回车。
