[English](README.md) | [简体中文](README.zh-CN.md)

# OpenTyless Rust CLI

OpenTyless is a small Rust voice-to-text CLI designed for desktop workflows where system shortcuts trigger commands and a user-level daemon does the heavy lifting.

It is built around:

- desktop shortcuts instead of global keyboard hooks
- a `systemd --user` daemon
- a local Unix socket for `start-record` / `stop-record` / `toggle-record`
- `ASR -> cleanup with Qwen Flash -> Markdown output`

For release and publishing workflows, see [Release Guide](docs/RELEASE.md).

## Release Tracks

This repository currently maintains two release tracks:

- `master`: the default Wayland-oriented track
- `x11`: the X11-focused track, including installer defaults tuned for X11 clipboard behavior and desktop startup

Use the track that matches the desktop environment you want to ship.

## Why This Architecture

On modern Linux desktops, especially GNOME Wayland, background apps are not reliable places to listen for global hotkeys.

So the project is structured like this:

- `opentyless-rs daemon`: stays in the background
- `opentyless-rs start-record`: starts recording through the daemon
- `opentyless-rs stop-record`: stops recording and runs the pipeline
- `opentyless-rs toggle-record`: the recommended shortcut target

The desktop environment is responsible for the shortcut. The CLI is responsible for recording and processing.

## Features

- daemon mode over a Unix socket
- one-shot mode with `once --seconds N`
- user-level `systemd` service installation
- GNOME shortcut installation
- tray menu support through AppIndicator / StatusNotifier
- clipboard copy after transcription
- desktop notifications for start, stop, and completion

## Installation

### Option A: one-shot installer

```bash
git clone <your-repo-url>
cd opentyless
chmod +x install.sh
./install.sh
```

The installer will:

- install common Debian/Ubuntu dependencies
- install Rust if it is missing
- build a release binary
- install `opentyless-rs` into `~/.local/bin`
- install the hotkey wrapper `opentyless-hotkey-toggle`
- generate `.env` from `.env.example` if needed
- install and optionally enable the `systemd --user` service
- install GNOME tray autostart

Common flags:

```bash
./install.sh --skip-deps
./install.sh --no-service
./install.sh --no-tray
./install.sh --install-shortcut --binding 'F8'
```

### Option B: manual installation

Install dependencies on Debian / Ubuntu:

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config curl git ffmpeg xclip wl-clipboard libasound2-dev libdbus-1-dev
```

Install Rust:

```bash
curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

Build:

```bash
cargo build --release
```

Install the binary:

```bash
mkdir -p ~/.local/bin
cp ./target/release/opentyless-rs ~/.local/bin/opentyless-rs
chmod +x ~/.local/bin/opentyless-rs
```

If `~/.local/bin` is not on your `PATH`, add:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## Configuration

Create the environment file:

```bash
cp .env.example .env
```

Minimum required configuration:

```env
DASHSCOPE_API_KEY=your_dashscope_key
```

Defaults already assume DashScope compatible mode:

- base URL: `https://dashscope.aliyuncs.com/compatible-mode/v1`
- ASR model: `qwen3-asr-flash`
- cleanup model: `qwen-flash`

Useful optional variables:

- `RECORDER_CMD`: override the recording command
- `AUTO_COPY_TO_CLIPBOARD`: enable or disable automatic copy
- `CLIPBOARD_COMMAND`: force `xclip`, `wl-copy`, or a custom command
- `ENABLE_NOTIFICATIONS`: enable or disable desktop notifications
- `OUTPUT_DIR`, `RUN_DIR`, `STATE_DIR`, `SOCKET_PATH`: override runtime paths

Default runtime directories are absolute XDG-style paths:

- `RUN_DIR`: `~/.local/state/opentyless/run`
- `STATE_DIR`: `~/.local/state/opentyless/state`
- `OUTPUT_DIR`: `~/.local/share/opentyless/outputs`
- `SOCKET_PATH`: `~/.local/state/opentyless/run/opentyless.sock`

## Quick Start

Check your environment:

```bash
opentyless-rs doctor
```

Start the daemon:

```bash
opentyless-rs daemon
```

Trigger recording:

```bash
opentyless-rs start-record
opentyless-rs stop-record
```

Or use the toggle form:

```bash
opentyless-rs toggle-record
```

Run a one-shot test:

```bash
opentyless-rs once --seconds 5
```

Check current state:

```bash
opentyless-rs status
```

## CLI Reference

### Recording and transcription

- `opentyless-rs daemon`
  Start the background daemon that owns the recording lifecycle.
- `opentyless-rs start-record`
  Ask the daemon to begin recording.
- `opentyless-rs stop-record`
  Ask the daemon to stop recording, return immediately, notify that transcription is starting, then run transcription and cleanup in the background.
- `opentyless-rs toggle-record`
  Toggle between start and stop; this is the best shortcut target.
- `opentyless-rs once --seconds 5`
  Record for a fixed number of seconds and process once without using the daemon.

### Status and diagnostics

- `opentyless-rs status`
  Print whether the daemon is running, the socket path, and the current state JSON.
- `opentyless-rs doctor`
  Print the effective recorder command, API configuration state, clipboard command, notification setting, and runtime directories.
- `opentyless-rs logs -n 50`
  Show recent `systemd --user` logs for the service.
- `opentyless-rs copy-last`
  Copy the last polished text back to the clipboard.

### Service management

- `opentyless-rs install-service --enable`
  Install the user service and optionally enable it immediately.
- `opentyless-rs uninstall-service`
  Stop and remove the user service.
- `opentyless-rs service-status`
  Show `systemctl --user status opentyless.service`.
- `opentyless-rs start`
  Start the user service.
- `opentyless-rs stop`
  Stop the user service.
- `opentyless-rs restart`
  Restart the user service.

### Desktop integration

- `opentyless-rs install-gnome-shortcut --binding '<Super>space'`
  Create a GNOME custom shortcut via `gsettings`.
- `opentyless-rs install-tray-autostart`
  Install a GNOME autostart entry for the tray.
- `opentyless-rs tray`
  Start the tray menu.

### Help

- `opentyless-rs --help`
  Show the top-level help.
- `opentyless-rs <command> --help`
  Show help for a specific subcommand.

## Autostart and Tray

Install and enable the user service:

```bash
opentyless-rs install-service --enable
```

Inspect the service:

```bash
opentyless-rs service-status
opentyless-rs logs -n 100
```

Install tray autostart:

```bash
opentyless-rs install-tray-autostart
```

Start the tray manually:

```bash
opentyless-rs tray
```

## Wayland and X11 Notes

### Wayland

- prefer desktop shortcuts bound to `toggle-record`
- clipboard defaults should usually point to `wl-copy`
- direct text injection into the focused input is intentionally not the default behavior

### X11

- the `x11` branch and `x11` release track tune installer defaults around `xclip`
- if needed, set `RECORDER_CMD` explicitly to the correct ALSA or Pulse device
- GNOME on X11 can still use the same daemon, service, tray, and shortcut workflow

## Troubleshooting

### Missing API key

If you see missing `ASR_API_KEY` or `FLASH_API_KEY`, make sure `.env` contains:

```env
DASHSCOPE_API_KEY=your_dashscope_key
```

### Cannot connect to the daemon

Reinstall or restart the service:

```bash
opentyless-rs install-service --enable
opentyless-rs restart
opentyless-rs status
```

### No recorder command found

Install `ffmpeg`, or set a custom command:

```env
RECORDER_CMD=ffmpeg -hide_banner -loglevel error -f pulse -i default -ac 1 -ar 16000
```

### Tray is visible but actions do nothing

Restart the service and the tray:

```bash
systemctl --user restart opentyless.service
pkill -f 'opentyless-rs tray' || true
setsid -f ~/.local/bin/opentyless-rs tray >/tmp/opentyless-tray.log 2>&1
```
