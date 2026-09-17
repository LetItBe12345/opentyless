const fs = require('node:fs');
const path = require('node:path');

const packageJson = require('../package.json');

function resolveRuntimeRoot() {
  return path.join(__dirname, 'runtime', 'current');
}

function resolveBinaryPath(name) {
  const candidates = [
    path.join(resolveRuntimeRoot(), name),
    path.join(__dirname, '..', 'target', 'release', name),
    path.join(__dirname, '..', 'target', 'debug', name)
  ];
  for (const fullPath of candidates) {
    if (fs.existsSync(fullPath)) {
      return fullPath;
    }
  }
  throw new Error(
    `未找到运行时文件: ${candidates[0]}。请重新执行 npm 安装，或在源码树中先编译 target/release/${name}。`
  );
}

module.exports = {
  resolveBinaryPath,
  resolveRuntimeRoot
};
