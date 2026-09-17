#!/usr/bin/env node

const fs = require('node:fs');
const fsp = require('node:fs/promises');
const https = require('node:https');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawnSync } = require('node:child_process');
const packageJson = require('../package.json');

const metadata = packageJson.opentyless;
const packageRoot = path.resolve(__dirname, '..');
const runtimeRoot = path.join(packageRoot, metadata.runtimeDir);
const runtimeCurrent = path.join(runtimeRoot, 'current');
const runtimeSource = metadata.runtimeSourceDir
  ? path.join(packageRoot, metadata.runtimeSourceDir)
  : '';

function buildReleaseTag() {
  const overrideTag = process.env.OPENTYLESS_RELEASE_TAG?.trim();
  if (overrideTag) {
    return overrideTag;
  }
  return metadata.defaultTagTemplate.replace('{version}', packageJson.version);
}

function buildArchiveName(tag) {
  return `${metadata.binaryName}-${tag}-${metadata.assetSuffix}.tar.gz`;
}

function buildChecksumUrl(tag) {
  return `https://github.com/${metadata.owner}/${metadata.repo}/releases/download/${tag}/SHA256SUMS.txt`;
}

function buildArchiveUrl(tag) {
  return `https://github.com/${metadata.owner}/${metadata.repo}/releases/download/${tag}/${buildArchiveName(tag)}`;
}

function download(url, destination) {
  return new Promise((resolve, reject) => {
    const request = https.get(
      url,
      {
        headers: {
          'User-Agent': `${packageJson.name}/${packageJson.version}`
        }
      },
      (response) => {
        if (response.statusCode >= 300 && response.statusCode < 400 && response.headers.location) {
          response.resume();
          download(response.headers.location, destination).then(resolve, reject);
          return;
        }

        if (response.statusCode !== 200) {
          response.resume();
          reject(new Error(`下载失败: ${url} -> HTTP ${response.statusCode}`));
          return;
        }

        const file = fs.createWriteStream(destination);
        response.pipe(file);
        file.on('finish', () => file.close(resolve));
        file.on('error', reject);
      }
    );

    request.on('error', reject);
  });
}

function sha256(filePath) {
  const hash = crypto.createHash('sha256');
  hash.update(fs.readFileSync(filePath));
  return hash.digest('hex');
}

async function maybeVerifyChecksum(tag, archivePath) {
  const checksumPath = path.join(os.tmpdir(), `${packageJson.name}-${Date.now()}-SHA256SUMS.txt`);
  try {
    await download(buildChecksumUrl(tag), checksumPath);
  } catch {
    return;
  }

  const archiveName = buildArchiveName(tag);
  const content = await fsp.readFile(checksumPath, 'utf8');
  const line = content
    .split('\n')
    .map((item) => item.trim())
    .find((item) => item.endsWith(archiveName));

  if (!line) {
    return;
  }

  const expected = line.split(/\s+/)[0];
  const actual = sha256(archivePath);
  if (expected !== actual) {
    throw new Error(`校验失败: 期望 ${expected}，实际 ${actual}`);
  }
}

async function installBundledRuntime() {
  if (!runtimeSource || !fs.existsSync(path.join(runtimeSource, metadata.binaryName))) {
    return false;
  }

  await fsp.mkdir(runtimeRoot, { recursive: true });
  await fsp.rm(runtimeCurrent, { recursive: true, force: true });
  await fsp.cp(runtimeSource, runtimeCurrent, { recursive: true });
  await fsp.chmod(path.join(runtimeCurrent, metadata.binaryName), 0o755);
  await fsp.chmod(path.join(runtimeCurrent, metadata.hotkeyName), 0o755);
  console.log(`[opentyless] 已安装随包附带的运行时到 ${runtimeCurrent}`);
  return true;
}

async function installFromGithubRelease() {
  if (process.platform !== 'linux' || process.arch !== 'x64') {
    throw new Error(`当前仅支持 linux x64，检测到 ${process.platform} ${process.arch}`);
  }

  if (spawnSync('tar', ['--version'], { stdio: 'ignore' }).status !== 0) {
    throw new Error('系统缺少 tar，无法解压 GitHub Release 产物');
  }

  const tag = buildReleaseTag();
  const archiveName = buildArchiveName(tag);
  const archiveUrl = buildArchiveUrl(tag);
  const tempRoot = await fsp.mkdtemp(path.join(os.tmpdir(), `${packageJson.name}-`));
  const archivePath = path.join(tempRoot, archiveName);
  const extractRoot = path.join(tempRoot, 'extract');

  console.log(`[opentyless] 下载 ${archiveUrl}`);
  await download(archiveUrl, archivePath);
  await maybeVerifyChecksum(tag, archivePath);
  await fsp.mkdir(extractRoot, { recursive: true });

  const extractResult = spawnSync('tar', ['-xzf', archivePath, '-C', extractRoot], {
    stdio: 'inherit'
  });
  if (extractResult.status !== 0) {
    throw new Error('解压 release 产物失败');
  }

  const packageDirName = `${metadata.binaryName}-${tag}-${metadata.assetSuffix}`;
  const unpackedRoot = path.join(extractRoot, packageDirName);
  const nextRoot = path.join(runtimeRoot, `${tag}-${Date.now()}`);

  await fsp.rm(nextRoot, { recursive: true, force: true });
  await fsp.mkdir(runtimeRoot, { recursive: true });
  await fsp.cp(unpackedRoot, nextRoot, { recursive: true });
  await fsp.chmod(path.join(nextRoot, metadata.binaryName), 0o755);
  await fsp.chmod(path.join(nextRoot, metadata.hotkeyName), 0o755);
  await fsp.rm(runtimeCurrent, { recursive: true, force: true });
  await fsp.cp(nextRoot, runtimeCurrent, { recursive: true });
  console.log(`[opentyless] 已从 GitHub Release 安装运行时到 ${runtimeCurrent}`);
}

async function main() {
  const installed = await installBundledRuntime();
  if (!installed) {
    await installFromGithubRelease();
  }
  console.log('[opentyless] 你现在可以执行 `opentyless doctor` 继续检查系统依赖。');
}

main().catch((error) => {
  console.error(`[opentyless] 安装失败: ${error.message}`);
  process.exit(1);
});
