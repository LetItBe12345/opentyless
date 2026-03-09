[English](RELEASE.md) | [简体中文](RELEASE.zh-CN.md)

# Release Guide

This repository currently ships two release tracks:

- Wayland track from `master`
- X11 track from `x11`

The default documentation entrypoint is [README.md](../README.md). The Chinese mirror is [README.zh-CN.md](../README.zh-CN.md).

## Branch Policy

- `master`
  The default branch. Use it for the Wayland-oriented build and release line.
- `x11`
  The X11-oriented branch. Use it for X11-focused releases and installer behavior.

Do not mix track-specific installer behavior into the wrong branch unless you intend to make it part of both tracks.

## Documentation Policy

Before publishing a release, keep these files in sync:

- [README.md](../README.md): English, default entrypoint
- [README.zh-CN.md](../README.zh-CN.md): Simplified Chinese mirror
- this release guide and its Chinese mirror

If a user-visible behavior changes, update docs first or in the same commit.

## Release Naming

Recommended naming:

- Wayland releases: `vX.Y.Z`
- X11 releases: `vX.Y.Z-x11.N`

Examples:

- `v0.2.0`
- `v0.2.0-x11.1`

## Pre-release Checklist

1. Make sure the target branch is clean except for intended changes.
2. Push the branch first.
3. Build the release binary locally.
4. Verify docs are updated in both languages.
5. Verify `install.sh` matches the target track behavior.
6. Confirm the generated binary starts and the daemon can run.

## Recommended Release Script

Use:

```bash
./scripts/release.sh --track wayland --tag v0.2.0
./scripts/release.sh --track x11 --tag v0.2.0-x11.1
```

What the script does:

- checks that you are on the expected branch
- builds a release binary
- stages a `dist/` package directory
- creates a `.tar.gz` artifact
- writes `SHA256SUMS.txt`
- optionally pushes the branch
- creates a GitHub Release through `gh`

## Manual Release Workflow

### Wayland release from `master`

```bash
git checkout master
git pull --ff-only
git push origin master
./scripts/release.sh --track wayland --tag v0.2.0
```

### X11 release from `x11`

```bash
git checkout x11
git pull --ff-only
git push origin x11
./scripts/release.sh --track x11 --tag v0.2.0-x11.1
```

## Artifacts

The release archive contains:

- `opentyless-rs`
- `opentyless-hotkey-toggle`
- `README.md`
- `README.zh-CN.md`
- `.env.example`

## Notes

- `gh` must be installed and authenticated.
- `dist/` is treated as a generated directory and should not be committed.
- If a release is meant for both tracks, cut two releases from the two branches instead of pretending one artifact covers both desktop assumptions.
