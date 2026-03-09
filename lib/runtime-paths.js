const fs = require('node:fs');
const path = require('node:path');

const packageJson = require('../package.json');

function resolveRuntimeRoot() {
  return path.join(__dirname, 'runtime', 'current');
}

function resolveBinaryPath(name) {
  const fullPath = path.join(resolveRuntimeRoot(), name);
  if (!fs.existsSync(fullPath)) {
    throw new Error(
      `未找到运行时文件: ${fullPath}。请重新执行 npm 安装，或检查 ${packageJson.name} 的 postinstall 是否成功。`
    );
  }
  return fullPath;
}

module.exports = {
  resolveBinaryPath,
  resolveRuntimeRoot
};
