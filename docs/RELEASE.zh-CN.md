[English](RELEASE.md) | [简体中文](RELEASE.zh-CN.md)

# 发布指南

当前仓库维护一条主发布线：

- 核心功能统一维护
- Wayland / X11 的差异通过环境检测和安装默认值处理

默认文档入口是 [README.md](../README.md)，中文镜像是 [README.zh-CN.md](../README.zh-CN.md)。

## 分支策略

- 推荐只维护一套主功能分支
- 如果历史上还有 `x11` 分支，可把它视为迁移期参考，不再作为长期功能分叉来源
- 新的行为差异优先通过环境检测、默认值和文档说明解决，而不是继续分裂发布线

## 文档策略

发版前，至少要保证这些文件同步：

- [README.md](../README.md)：英文默认入口
- [README.zh-CN.md](../README.zh-CN.md)：简体中文镜像
- 当前发布指南及其英文版本

只要用户可见行为发生变化，文档就应该一起更新。

## 版本命名

建议统一使用普通语义化版本：

- `vX.Y.Z`
- `vX.Y.Z-beta.N`

## 发布前检查

1. 确认目标分支工作区干净，只有你要发布的变更。
2. 先把分支推到远端。
3. 本地构建 release 二进制。
4. 确认中英文文档都已更新。
5. 确认 `install.sh` 和运行时默认值符合当前桌面环境自适应策略。
6. 确认生成的二进制可以启动，daemon 能正常运行。

## 推荐发布脚本

使用：

```bash
./scripts/release.sh --tag v0.2.0
```

脚本会：

- 构建 release 二进制
- 组装 `dist/` 发布目录
- 打包 `.tar.gz`
- 生成 `SHA256SUMS.txt`
- 可选地先推送分支
- 通过 `gh` 创建 GitHub Release

## 手动发布流程

```bash
git checkout master
git pull --ff-only
git push origin master
./scripts/release.sh --tag v0.2.0
```

## 发布产物

发布压缩包会包含：

- `opentyless-rs`
- `opentyless-hotkey-toggle`
- `README.md`
- `README.zh-CN.md`
- `.env.example`

## npm 分发

仓库根目录现在包含一个 `npm` 分发壳：

- `package.json`：npm 包定义
- `lib/postinstall.js`：安装时优先解包随 npm 发布的运行时，缺失时再回退到 GitHub Release
- `bin/opentyless.js`：启动 Rust 二进制
- `bin/opentyless-hotkey-toggle.js`：启动热键包装脚本

推荐顺序：

1. 在仓库根目录执行 `npm pack`，确认 `prepack` 已把 Rust 二进制打入 npm 包。
2. 发布 GitHub Release，作为回退下载来源。
3. 在仓库根目录执行 `npm publish`。

默认 npm 版本 `0.1.0` 会映射到 GitHub tag `v0.1.0`。如果你想安装其他 tag，可在安装时临时指定：

```bash
OPENTYLESS_RELEASE_TAG=v0.2.0 npm i -g opentyless@0.2.0
```

## 说明

- 机器上必须安装并登录 `gh`。
- `dist/` 是构建产物目录，不应提交进 git。
- 同一个 release 产物应通过环境检测同时兼容 Wayland 和 X11，而不是继续拆成两条长期发布线。
