#!/usr/bin/env node

const { spawn } = require('node:child_process');
const { resolveBinaryPath } = require('../lib/runtime-paths');

const scriptPath = resolveBinaryPath('opentyless-hotkey-toggle');

const child = spawn(scriptPath, process.argv.slice(2), {
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
  console.error(`[opentyless] 热键脚本启动失败: ${error.message}`);
  process.exit(1);
});
