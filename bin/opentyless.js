#!/usr/bin/env node

const { spawn } = require('node:child_process');
const { resolveBinaryPath } = require('../lib/runtime-paths');

const binaryPath = resolveBinaryPath('opentyless-rs');

const child = spawn(binaryPath, process.argv.slice(2), {
  stdio: 'inherit'
});

child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});

child.on('error', (error) => {
  console.error(`[opentyless] 启动失败: ${error.message}`);
  console.error('[opentyless] 如为首次安装失败，请重新执行 `npm i -g opentyless@latest`。');
  process.exit(1);
});
