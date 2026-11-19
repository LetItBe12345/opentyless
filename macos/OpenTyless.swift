import AVFoundation
import AppKit
import ApplicationServices
import Carbon
import Darwin
import QuartzCore

// A small native shell; the Rust CLI remains the only transcription pipeline.
let home = FileManager.default.homeDirectoryForCurrentUser
let configHome =
  ProcessInfo.processInfo.environment["XDG_CONFIG_HOME"].map { URL(fileURLWithPath: $0) }
  ?? home.appendingPathComponent(".config")
let configDir = configHome.appendingPathComponent("opentyless")
let cli = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/opentyless-rs")
let preferences = UserDefaults.standard

func runCLI(_ args: [String]) -> (Int32, String) {
  let process = Process()
  process.executableURL = cli
  process.arguments = args
  process.currentDirectoryURL = configDir
  let pipe = Pipe()
  process.standardOutput = pipe
  process.standardError = pipe
  do {
    try process.run()
    let output = pipe.fileHandleForReading.readDataToEndOfFile()
    process.waitUntilExit()
    return (process.terminationStatus, String(data: output, encoding: .utf8) ?? "")
  } catch { return (-1, error.localizedDescription) }
}

final class EnvStore {
  private var cachedValues: [String: String] = [:]
  private var lastModified: Date?
  private var fileExisted = false

  func get(_ name: String) -> String? {
    if let envVal = ProcessInfo.processInfo.environment[name]?.trimmingCharacters(in: .whitespaces),
      !envVal.isEmpty
    {
      return envVal
    }
    reloadIfNeeded()
    return cachedValues[name]
  }

  func invalidate() {
    lastModified = nil
  }

  private func reloadIfNeeded() {
    let envURL = configDir.appendingPathComponent(".env")
    let attrs = try? FileManager.default.attributesOfItem(atPath: envURL.path)
    let mtime = attrs?[.modificationDate] as? Date
    let exists = attrs != nil
    if exists == fileExisted && mtime == lastModified {
      return
    }
    fileExisted = exists
    lastModified = mtime
    guard exists, let text = try? String(contentsOf: envURL, encoding: .utf8) else {
      cachedValues = [:]
      return
    }
    cachedValues = Self.parse(text)
  }

  static func parse(_ text: String) -> [String: String] {
    var result: [String: String] = [:]
    for rawLine in text.components(separatedBy: .newlines) {
      var line = rawLine.trimmingCharacters(in: .whitespaces)
      if line.isEmpty || line.hasPrefix("#") { continue }
      if line.hasPrefix("export ") {
        line = String(line.dropFirst("export ".count)).trimmingCharacters(in: .whitespaces)
      }
      let parts = line.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
      guard parts.count == 2 else { continue }
      let key = parts[0].trimmingCharacters(in: .whitespaces)
      guard !key.isEmpty else { continue }
      var rawVal = parts[1].trimmingCharacters(in: .whitespaces)
      if (rawVal.hasPrefix("\"") && rawVal.hasSuffix("\"") && rawVal.count >= 2)
        || (rawVal.hasPrefix("'") && rawVal.hasSuffix("'") && rawVal.count >= 2)
      {
        rawVal = String(rawVal.dropFirst().dropLast())
      } else if let commentRange = rawVal.range(of: " #") {
        rawVal = String(rawVal[..<commentRange.lowerBound]).trimmingCharacters(in: .whitespaces)
      }
      if !rawVal.isEmpty {
        result[key] = rawVal
      }
    }
    return result
  }
}

let envStore = EnvStore()

func setting(_ name: String) -> String? {
  envStore.get(name)
}

func stateURL() -> URL {
  if let dir = setting("STATE_DIR") {
    return URL(fileURLWithPath: dir).appendingPathComponent("status.json")
  }
  let base =
    setting("XDG_STATE_HOME").map { URL(fileURLWithPath: $0) }
    ?? home.appendingPathComponent(".local/state")
  return base.appendingPathComponent("opentyless/state/status.json")
}

func outputURL() -> URL {
  if let dir = setting("OUTPUT_DIR") { return URL(fileURLWithPath: dir) }
  let base =
    setting("XDG_DATA_HOME").map { URL(fileURLWithPath: $0) }
    ?? home.appendingPathComponent(".local/share")
  return base.appendingPathComponent("opentyless/outputs")
}

func isDaemonProcessAlive(_ pid: Int32) -> Bool {
  guard pid > 0, kill(pid, 0) == 0 else { return false }
  var buffer = [CChar](repeating: 0, count: Int(MAXPATHLEN))
  let length = proc_pidpath(pid, &buffer, UInt32(buffer.count))
  guard length > 0 else { return true }
  let path = String(cString: buffer)
  return URL(fileURLWithPath: path).lastPathComponent == "opentyless-rs"
}

// Recorder mode is a child of the daemon. SIGINT/SIGTERM finalize the WAV.
// The application bundle supplies a stable microphone privacy identity.
final class RecorderSession {
  var recorder: AVAudioRecorder?
  var signals: [DispatchSourceSignal] = []
  func start(path: String) {
    umask(0o077)
    for number in [SIGINT, SIGTERM] {
      signal(number, SIG_IGN)
      let source = DispatchSource.makeSignalSource(signal: number, queue: .main)
      source.setEventHandler { [weak self] in
        self?.recorder?.stop()
        exit(0)
      }
      source.resume()
      signals.append(source)
    }
    let begin = { [self] in
      do {
        let settings: [String: Any] = [
          AVFormatIDKey: kAudioFormatLinearPCM,
          AVSampleRateKey: 16000, AVNumberOfChannelsKey: 1,
          AVLinearPCMBitDepthKey: 16, AVLinearPCMIsFloatKey: false,
          AVLinearPCMIsBigEndianKey: false,
        ]
        recorder = try AVAudioRecorder(url: URL(fileURLWithPath: path), settings: settings)
        guard recorder?.record() == true else {
          throw NSError(
            domain: "OpenTyless", code: 1, userInfo: [NSLocalizedDescriptionKey: "无法启动麦克风录音"])
        }
        try Data("ready".utf8).write(to: URL(fileURLWithPath: path + ".ready"), options: .atomic)
      } catch {
        try? Data(error.localizedDescription.utf8).write(to: URL(fileURLWithPath: path + ".error"))
        fputs("\(error)\n", stderr)
        exit(1)
      }
    }
    switch AVCaptureDevice.authorizationStatus(for: .audio) {
    case .authorized: begin()
    case .notDetermined:
      AVCaptureDevice.requestAccess(for: .audio) { allowed in
        DispatchQueue.main.async {
          if allowed { begin() } else { self.denied(path) }
        }
      }
    default: denied(path)
    }
  }
  func denied(_ path: String) {
    try? Data("麦克风权限未授权，请在系统设置 → 隐私与安全性 → 麦克风中允许 OpenTyless".utf8).write(
      to: URL(fileURLWithPath: path + ".error"))
    exit(1)
  }
}

struct HotkeyChoice {
  let name: String
  let key: UInt32
  let modifiers: UInt32
}
let hotkeys = [
  HotkeyChoice(name: "⌃⌥Space", key: UInt32(kVK_Space), modifiers: UInt32(controlKey | optionKey)),
  HotkeyChoice(name: "⌃⇧Space", key: UInt32(kVK_Space), modifiers: UInt32(controlKey | shiftKey)),
  HotkeyChoice(name: "F8", key: UInt32(kVK_F8), modifiers: 0),
  HotkeyChoice(name: "⌥Space", key: UInt32(kVK_Space), modifiers: UInt32(optionKey)),
]

enum PillKind {
  case listening
  case transcribing
  case success(String)
  case neutral(String)
  case warning(String)
}

final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
  var item: NSStatusItem!
  var dirWatchers: [DispatchSourceFileSystemObject] = []
  var watchedStateDirPath: String?
  var daemonExitWatcher: DispatchSourceProcess?
  var watchedDaemonPid: Int32 = 0
  var permissionTimer: Timer?
  var recordingClockTimer: Timer?
  var recordingStartedAt: Date?
  var refreshScheduled = false

  // Settings UI controls
  var settingsWindow: NSWindow?
  var headerStatusDot: NSView?
  var headerStatusLabel: NSTextField?
  var hotkeyPopup: NSPopUpButton?
  var mouseEnableSwitch: NSSwitch?
  var mouseSwapSwitch: NSSwitch?
  var micStatusLabel: NSTextField?
  var micActionButton: NSButton?
  var axStatusLabel: NSTextField?
  var axActionButton: NSButton?
  var loginSwitch: NSSwitch?
  var modelSummaryLabel: NSTextField?

  // Result preview UI
  var resultWindow: NSWindow?
  var resultText: NSTextView?
  var latestText = ""
  var observedEvent: String?

  // Sleek bottom capsule pill HUD
  var hud: NSPanel?
  var hudDot: NSView?
  var hudSpinner: NSProgressIndicator?
  var hudIcon: NSImageView?
  var hudLabel: NSTextField?
  var hudTimeLabel: NSTextField?
  var hudHide: DispatchWorkItem?

  var hotkey: EventHotKeyRef?
  var handler: EventHandlerRef?
  let mouseShortcuts = MouseShortcuts()
  var mouseShortcutError: String?
  var mouseShortcutsEnabled: Bool {
    preferences.object(forKey: "mouseShortcutsEnabled") as? Bool ?? true
  }
  var selected = 0
  var busy = false
  var menuTracking = false
  var renderedState = ""
  var hotkeyError: String?
  var statusText = "连接中…"
  var lastError: String?
  var recording = false
  var transcribing = false

  var apiConfigured: Bool {
    let hasAsr = setting("ASR_API_KEY") != nil || setting("DASHSCOPE_API_KEY") != nil
    let hasFlash =
      setting("FLASH_API_KEY") != nil || setting("DEEPSEEK_API_KEY") != nil
      || setting("DASHSCOPE_API_KEY") != nil
    return hasAsr && hasFlash
  }

  var activeModelSummary: String {
    let asr = setting("ASR_MODEL") ?? "qwen3-asr-flash"
    let defaultFlash = setting("DEEPSEEK_API_KEY") != nil ? "DeepSeek V4.1 Flash" : "Qwen Plus"
    let rawFlash = setting("FLASH_MODEL") ?? defaultFlash
    let flashDisplay =
      rawFlash.lowercased() == "deepseek-flash" || rawFlash.lowercased() == "deepseek-v4.1-flash"
      ? "DeepSeek V4.1 Flash" : rawFlash
    return "语音 \(asr)  ·  精修 \(flashDisplay)"
  }

  var loginEnabled: Bool {
    ["daemon", "tray"].allSatisfy {
      FileManager.default.fileExists(
        atPath: home.appendingPathComponent("Library/LaunchAgents/local.opentyless.\($0).plist")
          .path)
    }
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    NSApp.setActivationPolicy(.accessory)
    let others = NSRunningApplication.runningApplications(
      withBundleIdentifier: Bundle.main.bundleIdentifier ?? "local.opentyless.app")
    if others.contains(where: {
      $0.processIdentifier != getpid() && !$0.isTerminated
        && $0.launchDate ?? Date() < NSRunningApplication.current.launchDate ?? Date()
    }) {
      NSApp.terminate(nil)
      return
    }
    item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    item.button?.setAccessibilityLabel("OpenTyless")
    selected = min(max(preferences.integer(forKey: "hotkeyIndex"), 0), hotkeys.count - 1)
    var event = EventTypeSpec(
      eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
    let handlerCode = InstallEventHandler(
      GetApplicationEventTarget(),
      { _, _, pointer -> OSStatus in
        guard let pointer else { return OSStatus(eventNotHandledErr) }
        let delegate = Unmanaged<AppDelegate>.fromOpaque(pointer).takeUnretainedValue()
        DispatchQueue.main.async { delegate.toggle() }
        return noErr
      }, 1, &event, Unmanaged.passUnretained(self).toOpaque(), &handler)
    if handlerCode == noErr {
      registerHotkey(selected)
    } else {
      hotkeyError = "快捷键监听不可用（\(handlerCode)）"
    }
    mouseShortcuts.swapped = preferences.bool(forKey: "mouseButtonsSwapped")
    mouseShortcuts.onAction = { [weak self] action in
      DispatchQueue.main.async {
        guard let self, self.mouseShortcutsEnabled else { return }
        switch action {
        case .toggleRecording: self.toggle()
        case .paste: self.pasteClipboard()
        case .enter:
          for event in MouseShortcuts.enterEvents() { event.post(tap: .cgSessionEventTap) }
        }
      }
    }
    installDirectoryWatchers()
    DispatchQueue.global(qos: .utility).async { [weak self] in
      _ = runCLI(["start"])
      DispatchQueue.main.async { self?.scheduleRefresh() }
    }
    refresh()
    if !preferences.bool(forKey: "hasLaunched") {
      preferences.set(true, forKey: "hasLaunched")
      showSettings()
    }
  }

  func applicationDidBecomeActive(_ notification: Notification) {
    scheduleRefresh()
  }

  private func scheduleRefresh() {
    guard !refreshScheduled else { return }
    refreshScheduled = true
    DispatchQueue.main.async { [weak self] in
      guard let self else { return }
      self.refreshScheduled = false
      self.refresh()
    }
  }

  private func watchDirectory(_ url: URL) -> DispatchSourceFileSystemObject? {
    try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    let fd = open(url.path, O_EVTONLY)
    guard fd >= 0 else { return nil }
    let source = DispatchSource.makeFileSystemObjectSource(
      fileDescriptor: fd,
      eventMask: [.write, .rename, .delete, .attrib],
      queue: .main)
    source.setEventHandler { [weak self] in
      envStore.invalidate()
      self?.scheduleRefresh()
    }
    source.setCancelHandler { close(fd) }
    source.resume()
    return source
  }

  private func installDirectoryWatchers() {
    for watcher in dirWatchers { watcher.cancel() }
    dirWatchers.removeAll()
    let stateDir = stateURL().deletingLastPathComponent()
    watchedStateDirPath = stateDir.path
    for dir in [configDir, stateDir, home.appendingPathComponent("Library/LaunchAgents")] {
      if let watcher = watchDirectory(dir) {
        dirWatchers.append(watcher)
      }
    }
  }

  private func updateDaemonProcessWatcher(_ pid: Int32, running: Bool) {
    let targetPid = running ? pid : 0
    guard targetPid != watchedDaemonPid else { return }
    daemonExitWatcher?.cancel()
    daemonExitWatcher = nil
    watchedDaemonPid = targetPid
    guard targetPid > 0 else { return }
    let source = DispatchSource.makeProcessSource(
      identifier: targetPid, eventMask: .exit, queue: .main)
    source.setEventHandler { [weak self] in
      self?.scheduleRefresh()
    }
    source.resume()
    daemonExitWatcher = source
  }

  private func syncAccessibilityPollTimer() {
    let needsTrustWatch = mouseShortcutsEnabled && !AXIsProcessTrusted()
    if needsTrustWatch && permissionTimer == nil {
      let timer = Timer(timeInterval: 2.0, repeats: true) { [weak self] _ in
        guard let self else { return }
        if !self.mouseShortcutsEnabled || AXIsProcessTrusted() {
          self.scheduleRefresh()
        }
      }
      permissionTimer = timer
      RunLoop.main.add(timer, forMode: .common)
    } else if !needsTrustWatch, let timer = permissionTimer {
      timer.invalidate()
      permissionTimer = nil
    }
  }

  func registerHotkey(_ index: Int) {
    var candidate: EventHotKeyRef?
    let choice = hotkeys[index]
    let code = RegisterEventHotKey(
      choice.key, choice.modifiers, EventHotKeyID(signature: 0x4F54_594C, id: 1),
      GetApplicationEventTarget(), 0, &candidate)
    if code == noErr {
      if let hotkey { UnregisterEventHotKey(hotkey) }
      hotkey = candidate
      selected = index
      preferences.set(index, forKey: "hotkeyIndex")
      hotkeyError = nil
    } else {
      hotkeyError = "\(choice.name) 已被其他应用占用"
    }
  }

  func refresh() {
    let currentStateURL = stateURL()
    if currentStateURL.deletingLastPathComponent().path != watchedStateDirPath {
      installDirectoryWatchers()
    }
    updateMouseShortcuts()
    syncAccessibilityPollTimer()

    var state: [String: Any] = [:]
    if let data = try? Data(contentsOf: currentStateURL),
      let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    {
      state = value
    }
    let pid = state["pid"] as? Int32 ?? 0
    let running = isDaemonProcessAlive(pid)
    updateDaemonProcessWatcher(pid, running: running)

    let wasRecording = recording
    recording = running && state["recording"] as? Bool == true
    transcribing = running && state["last_event"] as? String == "transcribing"
    lastError = state["error"] as? String
    latestText = state["last_polished_text"] as? String ?? ""

    if recording && !wasRecording {
      startRecordingClock()
    } else if !recording && wasRecording {
      stopRecordingClock()
    }

    let event = state["last_event"] as? String ?? ""
    let eventID =
      "\(event)|\(state["audio_path"] as? String ?? "")|\(state["last_output_path"] as? String ?? "")|\(lastError ?? "")"
    if let previous = observedEvent, previous != eventID {
      switch event {
      case "recording_started":
        showPill(.listening, persistent: true)
      case "transcribing":
        showPill(.transcribing, persistent: true)
      case "pipeline_completed":
        if lastError != nil {
          showPill(.warning("复制失败"), duration: 2.2)
        } else {
          let copied = state["clipboard_copied"] as? Bool == true
          showPill(.success(copied ? "已复制" : "已完成"), duration: 1.3)
        }
      case "recording_saved":
        showPill(.success("音频已保存"), duration: 1.3)
      case "recording_cancelled":
        showPill(.neutral("已取消"), duration: 1.0)
      case "pipeline_failed", "command_failed":
        showPill(.warning("处理失败"), duration: 2.4)
      default:
        dismissPill()
      }
    }
    observedEvent = eventID

    let configured = apiConfigured
    statusText =
      !running
      ? "服务未运行"
      : !configured
        ? "待配置密钥"
        : recording
          ? "正在聆听" : transcribing ? "正在整理…" : lastError != nil ? "出现错误" : "就绪"

    let login = loginEnabled
    let axTrusted = AXIsProcessTrusted()
    let micAuth = AVCaptureDevice.authorizationStatus(for: .audio) == .authorized
    let fingerprint =
      "\(latestText)|\(statusText)|\(lastError ?? "")|\(selected)|\(hotkeyError ?? "")|\(login)|\(busy)|\(settingsWindow != nil)|\(configured)|\(mouseShortcutsEnabled)|\(mouseShortcuts.isRunning)|\(mouseShortcutError ?? "")|\(axTrusted)|\(micAuth)|\(mouseShortcuts.swapped)|\(activeModelSummary)"
    guard fingerprint != renderedState else { return }
    if !menuTracking { renderedState = fingerprint }

    item.button?.title = recording ? " ●" : transcribing ? " …" : ""
    item.button?.image = NSImage(
      systemSymbolName: recording ? "waveform.circle.fill" : "waveform",
      accessibilityDescription: "OpenTyless：\(statusText)")
    item.button?.image?.isTemplate = !recording
    item.button?.contentTintColor = recording ? .systemRed : nil
    item.button?.toolTip = "OpenTyless · \(statusText)（\(hotkeys[selected].name)）"

    updateSettingsControls(
      running: running, configured: configured, login: login, micAuth: micAuth, axTrusted: axTrusted
    )
    if item.menu == nil || !menuTracking { buildMenu() }
  }

  private func startRecordingClock() {
    recordingStartedAt = Date()
    recordingClockTimer?.invalidate()
    let timer = Timer(timeInterval: 0.5, repeats: true) { [weak self] _ in
      guard let self, self.recording, let started = self.recordingStartedAt else { return }
      let elapsed = max(0, Int(Date().timeIntervalSince(started)))
      self.hudTimeLabel?.stringValue = String(format: "%d:%02d", elapsed / 60, elapsed % 60)
    }
    recordingClockTimer = timer
    RunLoop.main.add(timer, forMode: .common)
  }

  private func stopRecordingClock() {
    recordingClockTimer?.invalidate()
    recordingClockTimer = nil
    recordingStartedAt = nil
  }

  // MARK: - Compact Product Menu

  func add(_ menu: NSMenu, _ title: String, _ action: Selector?, key: String = "", enabled: Bool = true)
    -> NSMenuItem
  {
    let entry = NSMenuItem(title: title, action: action, keyEquivalent: key)
    entry.target = self
    entry.isEnabled = enabled
    menu.addItem(entry)
    return entry
  }

  func buildMenu() {
    let menu = NSMenu()
    menu.autoenablesItems = false
    menu.delegate = self

    _ = add(menu, "OpenTyless · \(statusText)", nil, enabled: false)
    if let hotkeyError {
      _ = add(menu, "⚠️ \(hotkeyError)", #selector(showSettings))
    }
    if lastError != nil {
      _ = add(menu, "查看错误原因…", #selector(showError))
    }
    menu.addItem(.separator())

    if recording {
      _ = add(menu, "完成并整理", #selector(stopAndTranscribe), enabled: !busy)
      _ = add(menu, "只保存录音", #selector(saveRecording), enabled: !busy)
      _ = add(menu, "取消录音", #selector(cancel), enabled: !busy)
    } else {
      let label = transcribing ? "正在整理语音…" : "开始语音输入（\(hotkeys[selected].name)）"
      _ = add(menu, label, #selector(startRecording), enabled: !busy && !transcribing)
    }

    if !latestText.isEmpty {
      menu.addItem(.separator())
      let preview = latestText.replacingOccurrences(of: "\n", with: " ")
      let clipped = preview.count > 22 ? String(preview.prefix(22)) + "…" : preview
      _ = add(menu, "“\(clipped)”", nil, enabled: false)
      _ = add(menu, "复制上次结果", #selector(copyLast), enabled: !busy)
      _ = add(menu, "查看完整文本…", #selector(showResult))
    }

    menu.addItem(.separator())
    let shortcutItem = add(menu, "快捷键 · \(hotkeys[selected].name)", nil)
    let shortcutMenu = NSMenu()
    shortcutMenu.autoenablesItems = false
    for (index, choice) in hotkeys.enumerated() {
      let entry = add(shortcutMenu, choice.name, #selector(changeHotkeyFromMenu(_:)))
      entry.tag = index
      entry.state = (selected == index && hotkey != nil) ? .on : .off
    }
    shortcutItem.submenu = shortcutMenu

    let sideState =
      !mouseShortcutsEnabled
      ? "已关闭"
      : !AXIsProcessTrusted()
        ? "需授权"
        : (mouseShortcutError == nil && mouseShortcuts.isRunning ? "已启用" : "连接中")
    let mouseItem = add(menu, "鼠标侧键 · \(sideState)", nil)
    let mouseMenu = NSMenu()
    mouseMenu.autoenablesItems = false
    _ = add(mouseMenu, "上侧键：开始录音 / 停止并转写", nil, enabled: false)
    _ = add(mouseMenu, "下侧键：短按粘贴 · 长按 0.5s 回车发送", nil, enabled: false)
    mouseMenu.addItem(.separator())
    let enableEntry = add(mouseMenu, "启用鼠标侧键", #selector(toggleMouseShortcuts))
    enableEntry.state = mouseShortcutsEnabled ? .on : .off
    let swapEntry = add(mouseMenu, "交换上下侧键", #selector(swapMouseButtons))
    swapEntry.state = mouseShortcuts.swapped ? .on : .off
    _ = add(mouseMenu, "辅助功能权限设置…", #selector(mousePermission))
    mouseItem.submenu = mouseMenu

    _ = add(menu, "打开输出目录", #selector(openOutputs))
    _ = add(menu, "偏好设置…", #selector(showSettings), key: ",")

    let moreItem = add(menu, "高级", nil)
    let moreMenu = NSMenu()
    moreMenu.autoenablesItems = false
    _ = add(moreMenu, "编辑配置文件 (.env)…", #selector(openConfig))
    _ = add(
      moreMenu, "重新加载后台服务", #selector(restart),
      enabled: !busy && !recording && !transcribing)
    _ = add(moreMenu, "查看运行日志", #selector(openLogs))
    moreItem.submenu = moreMenu

    menu.addItem(.separator())
    _ = add(
      menu, "退出 OpenTyless", #selector(quit), key: "q",
      enabled: !busy && !recording && !transcribing)
    item.menu = menu
  }

  func menuWillOpen(_ menu: NSMenu) {
    menuTracking = true
    refresh()
  }

  func menuDidClose(_ menu: NSMenu) {
    menuTracking = false
    scheduleRefresh()
  }

  // MARK: - Sleek Bottom Capsule Pill HUD

  private func ensurePill() -> NSPanel {
    if let hud { return hud }
    let panel = NSPanel(
      contentRect: NSRect(x: 0, y: 0, width: 184, height: 40),
      styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
    panel.title = "OpenTyless"
    panel.isOpaque = false
    panel.backgroundColor = .clear
    panel.hasShadow = true
    panel.level = .statusBar
    panel.ignoresMouseEvents = true
    panel.isReleasedWhenClosed = false
    panel.appearance = NSAppearance(named: .darkAqua)
    panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]

    let effect = NSVisualEffectView(frame: NSRect(x: 0, y: 0, width: 184, height: 40))
    effect.autoresizingMask = [.width, .height]
    effect.material = .hudWindow
    effect.blendingMode = .behindWindow
    effect.state = .active
    effect.wantsLayer = true
    effect.layer?.cornerRadius = 20
    effect.layer?.masksToBounds = true
    effect.layer?.borderWidth = 0.8
    effect.layer?.borderColor = NSColor.white.withAlphaComponent(0.16).cgColor
    panel.contentView = effect

    let dot = NSView(frame: NSRect(x: 16, y: 15, width: 10, height: 10))
    dot.wantsLayer = true
    dot.layer?.cornerRadius = 5
    dot.layer?.backgroundColor = NSColor.systemRed.cgColor
    effect.addSubview(dot)

    let spinner = NSProgressIndicator(frame: NSRect(x: 13, y: 12, width: 16, height: 16))
    spinner.style = .spinning
    spinner.controlSize = .small
    spinner.isDisplayedWhenStopped = false
    effect.addSubview(spinner)

    let icon = NSImageView(frame: NSRect(x: 13, y: 11, width: 18, height: 18))
    icon.imageScaling = .scaleProportionallyUpOrDown
    effect.addSubview(icon)

    let label = NSTextField(labelWithString: "")
    label.font = .systemFont(ofSize: 13, weight: .medium)
    label.textColor = .white
    effect.addSubview(label)

    let timeLabel = NSTextField(labelWithString: "0:00")
    timeLabel.font = .monospacedDigitSystemFont(ofSize: 12, weight: .regular)
    timeLabel.textColor = NSColor.white.withAlphaComponent(0.65)
    timeLabel.alignment = .right
    effect.addSubview(timeLabel)

    hud = panel
    hudDot = dot
    hudSpinner = spinner
    hudIcon = icon
    hudLabel = label
    hudTimeLabel = timeLabel
    return panel
  }

  private func setRecordingPulse(_ active: Bool) {
    guard let layer = hudDot?.layer else { return }
    layer.removeAnimation(forKey: "pulse")
    guard active else { return }
    let pulse = CABasicAnimation(keyPath: "opacity")
    pulse.fromValue = 1.0
    pulse.toValue = 0.30
    pulse.duration = 0.75
    pulse.autoreverses = true
    pulse.repeatCount = .infinity
    pulse.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
    layer.add(pulse, forKey: "pulse")
  }

  func showPill(_ kind: PillKind, persistent: Bool = false, duration: TimeInterval = 1.4) {
    hudHide?.cancel()
    let panel = ensurePill()
    hudDot?.isHidden = true
    setRecordingPulse(false)
    hudSpinner?.stopAnimation(nil)
    hudIcon?.isHidden = true
    hudTimeLabel?.isHidden = true

    var text = ""
    var showTimer = false
    switch kind {
    case .listening:
      text = "正在聆听"
      showTimer = true
      hudDot?.isHidden = false
      setRecordingPulse(true)
      hudTimeLabel?.stringValue = "0:00"
      hudTimeLabel?.isHidden = false
    case .transcribing:
      text = "正在整理…"
      hudSpinner?.startAnimation(nil)
    case .success(let msg):
      text = msg
      hudIcon?.image = NSImage(systemSymbolName: "checkmark.circle.fill", accessibilityDescription: msg)
      hudIcon?.contentTintColor = .systemGreen
      hudIcon?.isHidden = false
    case .neutral(let msg):
      text = msg
      hudIcon?.image = NSImage(systemSymbolName: "xmark.circle.fill", accessibilityDescription: msg)
      hudIcon?.contentTintColor = NSColor.white.withAlphaComponent(0.75)
      hudIcon?.isHidden = false
    case .warning(let msg):
      text = msg
      hudIcon?.image = NSImage(
        systemSymbolName: "exclamationmark.circle.fill", accessibilityDescription: msg)
      hudIcon?.contentTintColor = .systemOrange
      hudIcon?.isHidden = false
    }

    hudLabel?.stringValue = text
    hudLabel?.sizeToFit()
    let labelWidth = ceil(hudLabel?.frame.width ?? 70)
    let timerExtra: CGFloat = showTimer ? 44 : 0
    let pillWidth = max(124, 38 + labelWidth + timerExtra + 18)
    let height: CGFloat = 40

    hudLabel?.frame = NSRect(x: 36, y: 10, width: labelWidth + 4, height: 20)
    hudTimeLabel?.frame = NSRect(x: pillWidth - 54, y: 10, width: 38, height: 19)

    let screen =
      NSScreen.screens.first(where: { NSMouseInRect(NSEvent.mouseLocation, $0.frame, false) })
      ?? NSScreen.main
    let origin: NSPoint
    if let screen {
      origin = NSPoint(
        x: round(screen.visibleFrame.midX - pillWidth / 2),
        y: round(screen.visibleFrame.minY + 34))
    } else {
      origin = .zero
    }

    panel.setFrame(NSRect(origin: origin, size: NSSize(width: pillWidth, height: height)), display: true)
    panel.alphaValue = 1.0
    panel.orderFrontRegardless()

    if !persistent {
      let work = DispatchWorkItem { [weak self] in self?.dismissPill() }
      hudHide = work
      DispatchQueue.main.asyncAfter(deadline: .now() + duration, execute: work)
    }
  }

  private func dismissPill() {
    guard let panel = hud, panel.isVisible else { return }
    setRecordingPulse(false)
    NSAnimationContext.runAnimationGroup(
      { context in
        context.duration = 0.22
        panel.animator().alphaValue = 0.0
      },
      completionHandler: { [weak panel] in
        if panel?.alphaValue == 0.0 {
          panel?.orderOut(nil)
        }
      })
  }

  // MARK: - Modern Native Settings Window

  func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool
  {
    showSettings()
    return true
  }

  private func makeCard(height: CGFloat) -> NSBox {
    let box = NSBox(frame: NSRect(x: 0, y: 0, width: 420, height: height))
    box.boxType = .custom
    box.cornerRadius = 12
    box.borderWidth = 1
    box.borderColor = NSColor.separatorColor.withAlphaComponent(0.45)
    box.fillColor = NSColor.controlBackgroundColor
    box.contentViewMargins = .zero
    return box
  }

  private func addDivider(to parent: NSView, y: CGFloat) {
    let line = NSBox(frame: NSRect(x: 16, y: y, width: 388, height: 1))
    line.boxType = .separator
    parent.addSubview(line)
  }

  @objc func showSettings() {
    if settingsWindow == nil {
      let window = NSWindow(
        contentRect: NSRect(x: 0, y: 0, width: 468, height: 446),
        styleMask: [.titled, .closable], backing: .buffered, defer: false)
      window.title = "OpenTyless 设置"
      window.isReleasedWhenClosed = false
      let root = NSView(frame: NSRect(x: 0, y: 0, width: 468, height: 446))
      window.contentView = root

      // Header
      let badge = NSView(frame: NSRect(x: 24, y: 384, width: 40, height: 40))
      badge.wantsLayer = true
      badge.layer?.cornerRadius = 10
      badge.layer?.backgroundColor = NSColor.controlAccentColor.withAlphaComponent(0.14).cgColor
      let badgeIcon = NSImageView(frame: NSRect(x: 9, y: 9, width: 22, height: 22))
      badgeIcon.image = NSImage(systemSymbolName: "waveform", accessibilityDescription: "OpenTyless")
      badgeIcon.contentTintColor = .controlAccentColor
      badge.addSubview(badgeIcon)
      root.addSubview(badge)

      let titleLabel = NSTextField(labelWithString: "OpenTyless")
      titleLabel.font = .systemFont(ofSize: 17, weight: .semibold)
      titleLabel.frame = NSRect(x: 74, y: 403, width: 160, height: 22)
      root.addSubview(titleLabel)

      let dot = NSView(frame: NSRect(x: 76, y: 388, width: 8, height: 8))
      dot.wantsLayer = true
      dot.layer?.cornerRadius = 4
      root.addSubview(dot)
      headerStatusDot = dot

      let subStatus = NSTextField(labelWithString: "")
      subStatus.font = .systemFont(ofSize: 12)
      subStatus.textColor = .secondaryLabelColor
      subStatus.frame = NSRect(x: 90, y: 383, width: 350, height: 17)
      root.addSubview(subStatus)
      headerStatusLabel = subStatus

      // Card 1: Shortcuts & Input
      let inputTitle = NSTextField(labelWithString: "触发与输入")
      inputTitle.font = .systemFont(ofSize: 12, weight: .semibold)
      inputTitle.textColor = .secondaryLabelColor
      inputTitle.frame = NSRect(x: 28, y: 352, width: 120, height: 16)
      root.addSubview(inputTitle)

      let inputCard = makeCard(height: 148)
      inputCard.frame = NSRect(x: 24, y: 198, width: 420, height: 148)
      root.addSubview(inputCard)
      let ic = inputCard.contentView!

      let hkLabel = NSTextField(labelWithString: "语音快捷键")
      hkLabel.font = .systemFont(ofSize: 13, weight: .medium)
      hkLabel.frame = NSRect(x: 16, y: 112, width: 140, height: 20)
      ic.addSubview(hkLabel)

      let popup = NSPopUpButton(frame: NSRect(x: 274, y: 108, width: 130, height: 26), pullsDown: false)
      for choice in hotkeys { popup.addItem(withTitle: choice.name) }
      popup.target = self
      popup.action = #selector(hotkeyPopupChanged(_:))
      ic.addSubview(popup)
      hotkeyPopup = popup
      addDivider(to: ic, y: 98)

      let msLabel = NSTextField(labelWithString: "鼠标侧键快捷操控")
      msLabel.font = .systemFont(ofSize: 13, weight: .medium)
      msLabel.frame = NSRect(x: 16, y: 69, width: 200, height: 19)
      ic.addSubview(msLabel)
      let msSub = NSTextField(labelWithString: "上侧键录音/转写 · 下侧键短按粘贴、长按 0.5s 发送")
      msSub.font = .systemFont(ofSize: 11)
      msSub.textColor = .secondaryLabelColor
      msSub.frame = NSRect(x: 16, y: 51, width: 280, height: 15)
      ic.addSubview(msSub)
      let msSwitch = NSSwitch(frame: NSRect(x: 364, y: 58, width: 40, height: 24))
      msSwitch.target = self
      msSwitch.action = #selector(toggleMouseShortcuts)
      ic.addSubview(msSwitch)
      mouseEnableSwitch = msSwitch
      addDivider(to: ic, y: 44)

      let swLabel = NSTextField(labelWithString: "反转鼠标上下侧键")
      swLabel.font = .systemFont(ofSize: 13, weight: .medium)
      swLabel.frame = NSRect(x: 16, y: 13, width: 200, height: 19)
      ic.addSubview(swLabel)
      let swSwitch = NSSwitch(frame: NSRect(x: 364, y: 10, width: 40, height: 24))
      swSwitch.target = self
      swSwitch.action = #selector(swapMouseButtons)
      ic.addSubview(swSwitch)
      mouseSwapSwitch = swSwitch

      // Card 2: System & Permissions
      let sysTitle = NSTextField(labelWithString: "权限与系统")
      sysTitle.font = .systemFont(ofSize: 12, weight: .semibold)
      sysTitle.textColor = .secondaryLabelColor
      sysTitle.frame = NSRect(x: 28, y: 172, width: 120, height: 16)
      root.addSubview(sysTitle)

      let sysCard = makeCard(height: 114)
      sysCard.frame = NSRect(x: 24, y: 52, width: 420, height: 114)
      root.addSubview(sysCard)
      let sc = sysCard.contentView!

      let micTitle = NSTextField(labelWithString: "麦克风权限")
      micTitle.font = .systemFont(ofSize: 13, weight: .medium)
      micTitle.frame = NSRect(x: 16, y: 83, width: 140, height: 19)
      sc.addSubview(micTitle)
      let micStat = NSTextField(labelWithString: "")
      micStat.font = .systemFont(ofSize: 12, weight: .medium)
      micStat.alignment = .right
      micStat.frame = NSRect(x: 260, y: 84, width: 144, height: 17)
      sc.addSubview(micStat)
      micStatusLabel = micStat
      let micBtn = NSButton(title: "授权…", target: self, action: #selector(microphone))
      micBtn.bezelStyle = .rounded
      micBtn.controlSize = .small
      micBtn.frame = NSRect(x: 334, y: 80, width: 72, height: 24)
      sc.addSubview(micBtn)
      micActionButton = micBtn
      addDivider(to: sc, y: 74)

      let axTitle = NSTextField(labelWithString: "辅助功能权限")
      axTitle.font = .systemFont(ofSize: 13, weight: .medium)
      axTitle.frame = NSRect(x: 16, y: 46, width: 160, height: 19)
      sc.addSubview(axTitle)
      let axStat = NSTextField(labelWithString: "")
      axStat.font = .systemFont(ofSize: 12, weight: .medium)
      axStat.alignment = .right
      axStat.frame = NSRect(x: 260, y: 47, width: 144, height: 17)
      sc.addSubview(axStat)
      axStatusLabel = axStat
      let axBtn = NSButton(title: "去开启…", target: self, action: #selector(mousePermission))
      axBtn.bezelStyle = .rounded
      axBtn.controlSize = .small
      axBtn.frame = NSRect(x: 334, y: 43, width: 72, height: 24)
      sc.addSubview(axBtn)
      axActionButton = axBtn
      addDivider(to: sc, y: 37)

      let lgTitle = NSTextField(labelWithString: "登录时自动启动")
      lgTitle.font = .systemFont(ofSize: 13, weight: .medium)
      lgTitle.frame = NSRect(x: 16, y: 9, width: 180, height: 19)
      sc.addSubview(lgTitle)
      let lgSwitch = NSSwitch(frame: NSRect(x: 364, y: 6, width: 40, height: 24))
      lgSwitch.target = self
      lgSwitch.action = #selector(toggleLogin)
      sc.addSubview(lgSwitch)
      loginSwitch = lgSwitch

      // Footer
      let modelLbl = NSTextField(labelWithString: "")
      modelLbl.font = .systemFont(ofSize: 11)
      modelLbl.textColor = .tertiaryLabelColor
      modelLbl.frame = NSRect(x: 26, y: 18, width: 256, height: 16)
      root.addSubview(modelLbl)
      modelSummaryLabel = modelLbl

      let cfgBtn = NSButton(title: "编辑配置…", target: self, action: #selector(openConfig))
      cfgBtn.bezelStyle = .rounded
      cfgBtn.controlSize = .small
      cfgBtn.frame = NSRect(x: 288, y: 14, width: 80, height: 24)
      root.addSubview(cfgBtn)

      let reloadBtn = NSButton(title: "重载服务", target: self, action: #selector(restart))
      reloadBtn.bezelStyle = .rounded
      reloadBtn.controlSize = .small
      reloadBtn.frame = NSRect(x: 372, y: 14, width: 72, height: 24)
      root.addSubview(reloadBtn)

      window.center()
      settingsWindow = window
    }
    refresh()
    NSApp.activate(ignoringOtherApps: true)
    settingsWindow?.makeKeyAndOrderFront(nil)
  }

  private func updateSettingsControls(
    running: Bool, configured: Bool, login: Bool, micAuth: Bool, axTrusted: Bool
  ) {
    guard settingsWindow != nil else { return }
    let dotColor: NSColor =
      !running ? .systemGray : !configured ? .systemOrange : lastError != nil ? .systemOrange : .systemGreen
    headerStatusDot?.layer?.backgroundColor = dotColor.cgColor
    headerStatusLabel?.stringValue =
      configured ? statusText : "未检测到 API 密钥，请点击右下角「编辑配置…」填写"

    hotkeyPopup?.selectItem(at: selected)
    mouseEnableSwitch?.state = mouseShortcutsEnabled ? .on : .off
    mouseSwapSwitch?.state = mouseShortcuts.swapped ? .on : .off
    mouseSwapSwitch?.isEnabled = mouseShortcutsEnabled

    micStatusLabel?.stringValue = "已授权"
    micStatusLabel?.textColor = .systemGreen
    micStatusLabel?.isHidden = !micAuth
    micActionButton?.isHidden = micAuth

    axStatusLabel?.stringValue = "已授权"
    axStatusLabel?.textColor = .systemGreen
    axStatusLabel?.isHidden = !axTrusted
    axActionButton?.isHidden = axTrusted

    loginSwitch?.state = login ? .on : .off
    loginSwitch?.isEnabled = !busy && !recording && !transcribing
    modelSummaryLabel?.stringValue = activeModelSummary
  }

  @objc func hotkeyPopupChanged(_ sender: NSPopUpButton) {
    let idx = sender.indexOfSelectedItem
    guard idx >= 0 && idx < hotkeys.count else { return }
    registerHotkey(idx)
    if let hotkeyError {
      alert("快捷键冲突", hotkeyError)
    }
    renderedState = ""
    refresh()
  }

  // MARK: - Actions & Result Window

  func updateMouseShortcuts() {
    guard mouseShortcutsEnabled && AXIsProcessTrusted() else {
      mouseShortcuts.stop()
      mouseShortcutError = nil
      return
    }
    if !mouseShortcuts.isRunning {
      mouseShortcutError = mouseShortcuts.start() ? nil : "侧键监听未就绪"
    }
  }

  @objc func toggleMouseShortcuts() {
    preferences.set(!mouseShortcutsEnabled, forKey: "mouseShortcutsEnabled")
    renderedState = ""
    refresh()
    if mouseShortcutsEnabled && !AXIsProcessTrusted() { mousePermission() }
  }

  @objc func swapMouseButtons() {
    mouseShortcuts.swapped.toggle()
    preferences.set(mouseShortcuts.swapped, forKey: "mouseButtonsSwapped")
    renderedState = ""
    refresh()
  }

  @objc func mousePermission() {
    let options =
      [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
    _ = AXIsProcessTrustedWithOptions(options)
    NSWorkspace.shared.open(
      URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")!)
  }

  func pasteClipboard() {
    guard AXIsProcessTrusted() else {
      mousePermission()
      return
    }
    for event in MouseShortcuts.pasteEvents() { event.post(tap: .cgSessionEventTap) }
  }

  func alert(_ title: String, _ body: String) {
    NSApp.activate(ignoringOtherApps: true)
    let alert = NSAlert()
    alert.messageText = title
    alert.informativeText = body
    alert.runModal()
  }

  func perform(_ args: [String], completion: (() -> Void)? = nil) {
    guard !busy else { return }
    busy = true
    DispatchQueue.global(qos: .userInitiated).async {
      let result = runCLI(args)
      DispatchQueue.main.async {
        self.busy = false
        if result.0 != 0 { self.alert("操作未完成", result.1) } else { completion?() }
        self.refresh()
      }
    }
  }

  @objc func toggle() {
    guard !busy && !transcribing else { return }
    if recording {
      perform(["toggle-record"])
      return
    }
    startRecording()
  }

  @objc func changeHotkeyFromMenu(_ sender: NSMenuItem) {
    let idx = sender.tag
    guard idx >= 0 && idx < hotkeys.count else { return }
    if idx != selected || hotkey == nil { registerHotkey(idx) }
    if let hotkeyError { alert("快捷键冲突", hotkeyError) }
    renderedState = ""
    refresh()
  }

  @objc func startRecording() {
    guard !busy && !recording && !transcribing else { return }
    startWithMicrophone("start-record")
  }

  @objc func stopAndTranscribe() { perform(["stop-record"]) }

  @objc func saveRecording() { perform(["save-record"]) }

  func startWithMicrophone(_ command: String) {
    if AVCaptureDevice.authorizationStatus(for: .audio) == .authorized {
      perform([command])
      return
    }
    busy = true
    AVCaptureDevice.requestAccess(for: .audio) { allowed in
      DispatchQueue.main.async {
        self.busy = false
        if allowed { self.perform([command]) }
        else { self.alert("需要麦克风权限", "请在系统设置 → 隐私与安全性 → 麦克风中允许 OpenTyless。") }
        self.renderedState = ""
        self.refresh()
      }
    }
  }

  @objc func copyLast() {
    perform(["copy-last"]) { self.showPill(.success("已复制"), duration: 1.2) }
  }

  @objc func cancel() {
    perform(["cancel-record"])
  }

  @objc func showResult() {
    if resultWindow == nil {
      let window = NSWindow(
        contentRect: NSRect(x: 0, y: 0, width: 520, height: 340),
        styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
      window.title = "最近结果"
      window.isReleasedWhenClosed = false
      window.minSize = NSSize(width: 380, height: 240)

      let scroll = NSScrollView(frame: NSRect(x: 20, y: 64, width: 480, height: 256))
      scroll.hasVerticalScroller = true
      scroll.drawsBackground = true
      scroll.wantsLayer = true
      scroll.layer?.cornerRadius = 8
      scroll.layer?.borderWidth = 1
      scroll.layer?.borderColor = NSColor.separatorColor.withAlphaComponent(0.5).cgColor
      scroll.autoresizingMask = [.width, .height]

      let text = NSTextView(frame: scroll.bounds)
      text.isEditable = false
      text.isSelectable = true
      text.isRichText = false
      text.font = .systemFont(ofSize: 14)
      text.textContainerInset = NSSize(width: 12, height: 12)
      text.isVerticallyResizable = true
      text.autoresizingMask = [.width]
      text.textContainer?.widthTracksTextView = true
      scroll.documentView = text
      window.contentView?.addSubview(scroll)
      resultText = text

      let copy = NSButton(title: "复制全文", target: self, action: #selector(copyPreview))
      copy.frame = NSRect(x: 400, y: 18, width: 100, height: 30)
      copy.bezelStyle = .rounded
      copy.keyEquivalent = "\r"
      copy.autoresizingMask = [.minXMargin, .maxYMargin]
      window.contentView?.addSubview(copy)

      window.center()
      resultWindow = window
    }
    resultText?.string = latestText
    NSApp.activate(ignoringOtherApps: true)
    resultWindow?.makeKeyAndOrderFront(nil)
  }

  @objc func copyPreview() {
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(resultText?.string ?? "", forType: .string)
    showPill(.success("已复制"), duration: 1.2)
  }

  @objc func restart() {
    envStore.invalidate()
    perform(["restart"]) { self.showPill(.success("服务已重载"), duration: 1.2) }
  }

  @objc func showError() { alert("错误详情", lastError ?? "无错误") }

  @objc func openConfig() {
    let url = configDir.appendingPathComponent(".env")
    if let editor = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.TextEdit")
    {
      NSWorkspace.shared.open(
        [url], withApplicationAt: editor, configuration: NSWorkspace.OpenConfiguration())
    } else {
      NSWorkspace.shared.open(url)
    }
  }

  @objc func openOutputs() {
    let url = outputURL()
    try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    NSWorkspace.shared.open(url)
  }

  @objc func openLogs() {
    NSWorkspace.shared.open(home.appendingPathComponent("Library/Logs/opentyless"))
  }

  @objc func microphone() {
    AVCaptureDevice.requestAccess(for: .audio) { allowed in
      DispatchQueue.main.async {
        self.renderedState = ""
        self.refresh()
        if !allowed {
          NSWorkspace.shared.open(
            URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")!)
        }
      }
    }
  }

  @objc func toggleLogin() {
    guard !busy else { return }
    let enabled = loginEnabled
    busy = true
    DispatchQueue.global(qos: .userInitiated).async {
      var result: (Int32, String) = (0, "")
      if enabled {
        do {
          for name in ["daemon", "tray"] {
            try FileManager.default.removeItem(
              at: home.appendingPathComponent("Library/LaunchAgents/local.opentyless.\(name).plist")
            )
          }
        } catch { result = (-1, error.localizedDescription) }
      } else {
        result = runCLI(["install-service", "--enable"])
        if result.0 == 0 { result = runCLI(["install-tray-service"]) }
      }
      DispatchQueue.main.async {
        self.busy = false
        if result.0 != 0 {
          self.alert("自启动设置失败", result.1)
        }
        self.renderedState = ""
        self.refresh()
      }
    }
  }

  @objc func quit() { perform(["stop"]) { NSApp.terminate(nil) } }

  func applicationWillTerminate(_ notification: Notification) {
    stopRecordingClock()
    permissionTimer?.invalidate()
    daemonExitWatcher?.cancel()
    for watcher in dirWatchers { watcher.cancel() }
    mouseShortcuts.stop()
    if let hotkey { UnregisterEventHotKey(hotkey) }
    if let handler { RemoveEventHandler(handler) }
  }
}

@main
enum OpenTyless {
  static func main() {
    if CommandLine.arguments.count == 3 && CommandLine.arguments[1] == "--record" {
      let session = RecorderSession()
      session.start(path: CommandLine.arguments[2])
      withExtendedLifetime(session) { RunLoop.main.run() }
    } else {
      let app = NSApplication.shared
      let delegate = AppDelegate()
      app.delegate = delegate
      withExtendedLifetime(delegate) { app.run() }
    }
  }
}
