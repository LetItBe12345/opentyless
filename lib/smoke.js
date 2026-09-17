#!/usr/bin/env node

const { spawnSync } = require('node:child_process');
const { resolveBinaryPath } = require('./runtime-paths');

const binary = resolveBinaryPath('opentyless-rs');
const result = spawnSync(binary, ['--help'], {
  stdio: 'inherit'
});

process.exit(result.status ?? 1);
