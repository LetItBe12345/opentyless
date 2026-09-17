[English](RELEASE.md) | [简体中文](RELEASE.zh-CN.md)

# Release Guide

This repository now ships one primary release line:

- the core feature set is maintained in one place
- Wayland / X11 differences are handled through environment detection and installer defaults

The default documentation entrypoint is [README.md](../README.md). The Chinese mirror is [README.zh-CN.md](../README.zh-CN.md).

## Branch Policy

- Prefer one main feature branch
- If an old `x11` branch still exists, treat it as migration history rather than an ongoing long-term feature split
- New behavior differences should be handled by environment detection, defaults, and docs instead of separate release tracks

## Documentation Policy

Before publishing a release, keep these files in sync:

- [README.md](../README.md): English, default entrypoint
- [README.zh-CN.md](../README.zh-CN.md): Simplified Chinese mirror
- this release guide and its Chinese mirror

If a user-visible behavior changes, update docs first or in the same commit.

## Release Naming

Recommended naming:

- `vX.Y.Z`
- `vX.Y.Z-beta.N`

## Pre-release Checklist

1. Make sure the target branch is clean except for intended changes.
2. Push the branch first.
3. Build the release binary locally.
4. Verify docs are updated in both languages.
5. Verify `install.sh` and runtime defaults still match the environment-adaptive behavior.
6. Confirm the generated binary starts and the daemon can run.

## Recommended Release Script

Use:

```bash
./scripts/release.sh --tag v0.2.0
```

What the script does:

- builds a release binary
- stages a `dist/` package directory
- creates a `.tar.gz` artifact
- writes `SHA256SUMS.txt`
- optionally pushes the branch
- creates a GitHub Release through `gh`

## Manual Release Workflow

```bash
git checkout master
git pull --ff-only
git push origin master
./scripts/release.sh --tag v0.2.0
```

## Artifacts

The release archive contains:

- `opentyless-rs`
- `opentyless-hotkey-toggle`
- `README.md`
- `README.zh-CN.md`
- `.env.example`

## npm Distribution

The repository root now contains a thin npm distribution layer:

- `package.json`: npm package manifest
- `lib/postinstall.js`: installs the bundled runtime first, then falls back to GitHub Releases if needed
- `bin/opentyless.js`: forwards to the Rust binary
- `bin/opentyless-hotkey-toggle.js`: forwards to the hotkey wrapper script

Recommended flow:

1. Run `npm pack` first and confirm `prepack` bundled the Rust runtime into the npm tarball.
2. Publish the GitHub Release as a fallback download source.
3. Run `npm publish` from the repository root.

By default npm version `0.1.0` maps to GitHub tag `v0.1.0`. To target a different tag during install, use:

```bash
OPENTYLESS_RELEASE_TAG=v0.2.0 npm i -g opentyless@0.2.0
```

## Notes

- `gh` must be installed and authenticated.
- `dist/` is treated as a generated directory and should not be committed.
- A single release artifact should adapt to both Wayland and X11 through environment-aware defaults rather than long-term split release tracks.
