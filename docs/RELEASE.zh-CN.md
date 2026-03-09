[English](RELEASE.md) | [简体中文](RELEASE.zh-CN.md)

# 发布指南

当前仓库维护两条发布线：

- `master` 对应 Wayland 取向发布线
- `x11` 对应 X11 取向发布线

默认文档入口是 [README.md](../README.md)，中文镜像是 [README.zh-CN.md](../README.zh-CN.md)。

## 分支策略

- `master`
  默认分支，用于 Wayland 取向构建和发布。
- `x11`
  面向 X11 的分支，用于 X11 取向安装和发布。

除非你明确希望两条发布线共享同样行为，否则不要把某一条线的安装默认值混到另一条线上。

## 文档策略

发版前，至少要保证这些文件同步：

- [README.md](../README.md)：英文默认入口
- [README.zh-CN.md](../README.zh-CN.md)：简体中文镜像
- 当前发布指南及其英文版本

只要用户可见行为发生变化，文档就应该一起更新。

## 版本命名

建议：

- Wayland 版本：`vX.Y.Z`
- X11 版本：`vX.Y.Z-x11.N`

示例：

- `v0.2.0`
- `v0.2.0-x11.1`

## 发布前检查

1. 确认目标分支工作区干净，只有你要发布的变更。
2. 先把分支推到远端。
3. 本地构建 release 二进制。
4. 确认中英文文档都已更新。
5. 确认 `install.sh` 符合当前发布线预期。
6. 确认生成的二进制可以启动，daemon 能正常运行。

## 推荐发布脚本

使用：

```bash
./scripts/release.sh --track wayland --tag v0.2.0
./scripts/release.sh --track x11 --tag v0.2.0-x11.1
```

脚本会：

- 检查当前分支是否符合发布线
- 构建 release 二进制
- 组装 `dist/` 发布目录
- 打包 `.tar.gz`
- 生成 `SHA256SUMS.txt`
- 可选地先推送分支
- 通过 `gh` 创建 GitHub Release

## 手动发布流程

### 从 `master` 发布 Wayland 版本

```bash
git checkout master
git pull --ff-only
git push origin master
./scripts/release.sh --track wayland --tag v0.2.0
```

### 从 `x11` 发布 X11 版本

```bash
git checkout x11
git pull --ff-only
git push origin x11
./scripts/release.sh --track x11 --tag v0.2.0-x11.1
```

## 发布产物

发布压缩包会包含：

- `opentyless-rs`
- `opentyless-hotkey-toggle`
- `README.md`
- `README.zh-CN.md`
- `.env.example`

## 说明

- 机器上必须安装并登录 `gh`。
- `dist/` 是构建产物目录，不应提交进 git。
- 如果同一版本要同时支持 Wayland 和 X11，请分别从两个分支各发一个 Release，不要假设单个产物能覆盖两种桌面假设。
