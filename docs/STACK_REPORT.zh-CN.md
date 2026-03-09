# OpenTyless npm 化改造报告

## 这次做了什么

我为仓库增加了一层 npm 分发外壳，使它具备类似 `codex` 的安装方式：

```bash
npm i -g opentyless@latest
```

核心思路不是把 Rust 改写成 Node，而是：

- Rust 继续负责真正的 CLI、daemon、录音、转写和托盘逻辑
- Node/npm 只负责发布分发、安装到本地，以及命令转发

## 使用的技术栈

### 1. Rust

核心程序仍然是 Rust：

- `clap`：命令行参数解析
- `reqwest`：调用 ASR 与整理接口
- `serde` / `serde_json`：配置、状态、请求与响应序列化
- `ksni`：托盘支持
- `anyhow`：错误处理
- `chrono`：时间处理
- `dotenvy`：环境变量加载

Rust 部分负责：

- daemon 常驻
- Unix socket 通信
- 录音生命周期管理
- ASR + Flash 文本整理
- 剪贴板复制
- 桌面通知
- systemd 用户服务管理
- GNOME 快捷键与托盘逻辑

### 2. Node.js / npm

新增的 npm 层使用了 Node 原生能力，没有额外第三方依赖：

- `fs` / `fs.promises`：文件写入、复制与运行时目录管理
- `child_process`：启动 Rust 二进制，并在回退模式下调用 `tar` 解压
- `https`：回退模式下下载 GitHub Release 产物
- `crypto`：回退模式下做 SHA256 校验
- `path` / `os`：路径和临时目录处理

Node 部分负责：

- `prepack` 在发布前把 Rust release 二进制打入 npm 包
- `postinstall` 把随包运行时落到本地目录
- 缺失时回退下载 GitHub Release
- 提供 `opentyless` 与 `opentyless-hotkey-toggle` 启动入口

### 3. GitHub Release

GitHub Release 现在是二进制回退源：

- 通过 `scripts/release.sh` 生成 `.tar.gz`
- 附带 `SHA256SUMS.txt`
- 当 npm 包内没有附带运行时时，安装器才会按版本去 Release 下载对应产物

这意味着你后续的发布链路会变成：

1. Rust 构建 release
2. 上传 GitHub Release
3. npm publish 发布“分发壳”

## 为什么这套方案像 codex

因为它遵循的是同一种分层思路：

- 真正高性能逻辑由原生程序承担
- npm 只作为安装器、分发器和升级入口
- 用户体验是 `npm i -g ...@latest`

所以“通过 npm 安装”并不意味着“后端必须用 Node 写”。

## 当前落地文件

- `package.json`
- `bin/opentyless.js`
- `bin/opentyless-hotkey-toggle.js`
- `lib/postinstall.js`
- `lib/runtime-paths.js`
- `lib/smoke.js`
- `docs/NPM.zh-CN.md`
- `docs/STACK_REPORT.zh-CN.md`

## 当前限制

- 还没有真正执行 `npm publish`，因为这需要你的 npm 账号和发布权限
- 当前 npm 包默认面向 `linux x64`
- X11 / Wayland 双轨目前是“同一个分发壳 + 可覆盖 tag”，还没拆成两个独立 npm 包

## 我建议你下一步这样做

1. 先把 Rust 版本号和 GitHub tag 规划好
2. 先发一个新的 Wayland Release
3. 在本地执行 `npm pack` 自测
4. 用你的 npm 账号执行 `npm publish`
5. 如果 X11 需要长期并行，单独再发一个 `opentyless-x11` 包
