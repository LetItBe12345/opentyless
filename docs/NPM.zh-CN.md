# npm 分发说明

这套方案的目标是：

- 核心程序继续使用 Rust 编写
- 用户安装与升级体验改成 `npm i -g opentyless@latest`
- npm 包优先直接携带预编译产物；必要时再回退下载 GitHub Release

## 结构

- `package.json`：npm 包入口、版本与二进制映射
- `lib/postinstall.js`：安装时优先展开随包运行时，缺失时再下载 Release 压缩包并校验 `SHA256SUMS.txt`
- `bin/opentyless.js`：把命令转发给下载下来的 `opentyless-rs`
- `bin/opentyless-hotkey-toggle.js`：转发给 `opentyless-hotkey-toggle`
- `lib/runtime/current/`：安装后的运行时目录

## 安装流程

1. 用户执行 `npm i -g opentyless@latest`
2. npm 运行 `prepack` 时已把 Rust 二进制打入 npm 包
3. 安装阶段运行 `postinstall`
4. `postinstall` 优先把随包运行时复制到 `lib/runtime/current/`
5. 如果随包运行时缺失，再按 npm 版本推导 GitHub tag，并从 Release 下载回退产物
6. 用户执行 `opentyless` 时，由 Node 启动器转发给 Rust 二进制

## 当前约束

- 当前仅支持 `linux x64`
- 本地需要系统自带 `tar`
- `npm` 只能分发你的程序；`ffmpeg`、`xclip`、`wl-copy`、`systemd` 这些系统依赖仍需用户自己安装
- 默认会把 npm 版本 `0.1.0` 映射到 GitHub tag `v0.1.0`

## 发布顺序

推荐每次按这个顺序发布：

```bash
git push origin master
./scripts/release.sh --track wayland --tag v0.2.0 --push-first
npm pack
npm publish
```

如果你要发 X11 版本，可以：

- 单独维护一个 npm 包名，例如 `opentyless-x11`
- 或在安装时通过环境变量覆盖目标 tag

示例：

```bash
OPENTYLESS_RELEASE_TAG=v0.2.0-x11.1 npm i -g opentyless@0.2.0
```

## 后续可增强项

- 支持 `arm64`
- 支持 Windows / macOS 平台分发
- 支持多包策略：`opentyless` 和 `opentyless-x11`
- 在 GitHub Actions 中自动联动 `npm publish`
- 增加自检命令，显示当前 npm 包对应的 GitHub tag 与本地运行时路径
