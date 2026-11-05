import AVFoundation
import AppKit
import ApplicationServices
import Carbon

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

func setting(_ name: String) -> String? {
  if let value = ProcessInfo.processInfo.environment[name] { return value.isEmpty ? nil : value }
  let text =
    (try? String(contentsOf: configDir.appendingPathComponent(".env"), encoding: .utf8)) ?? ""
  for line in text.components(separatedBy: .newlines) {
    let parts = line.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
    guard parts.count == 2, parts[0].trimmingCharacters(in: .whitespaces) == name else { continue }
    let value = String(parts[1]).trimmingCharacters(in: CharacterSet(charactersIn: "\"' "))
    return value.isEmpty ? nil : value
  }
  return nil
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

final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
  var item: NSStatusItem!
  var timer: Timer?
  var settingsWindow: NSWindow?
  var settingsLabel: NSTextField?
  var resultWindow: NSWindow?
  var resultText: NSTextView?
  var latestText = ""
  var observedEvent: String?
  var hud: NSPanel?
  var hudHide: DispatchWorkItem?
  var hotkey: EventHotKeyRef?
  var handler: EventHandlerRef?
  let mouseShortcuts = MouseShortcuts()
  var mouseShortcutError: String?
  var mouseShortcutsEnabled: Bool {
    preferences.object(forKey: "mouseShortcutsEnabled") as? Bool ?? true
  }
  var mouseShortcutStatus: String {
    if !mouseShortcutsEnabled { return "已关闭" }
    if !AXIsProcessTrusted() { return "请授权辅助功能" }
    return mouseShortcutError ?? (mouseShortcuts.isRunning ? "上侧键录音 / 下侧键粘贴" : "正在连接")
  }
  var selected = 0
  var busy = false
  var menuTracking = false
  var renderedState = ""
  var hotkeyError: String?
  var statusText = "正在连接后台服务…"
  var lastError: String?
  var recording = false
  var transcribing = false
  var apiConfigured: Bool {
    setting("DASHSCOPE_API_KEY") != nil
      || (setting("ASR_API_KEY") != nil && setting("FLASH_API_KEY") != nil)
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
    // Avoid duplicate status items if launched through both Finder and launchd.
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
      hotkeyError = "快捷键事件监听失败（\(handlerCode)）"
    }
    mouseShortcuts.swapped = preferences.bool(forKey: "mouseButtonsSwapped")
    mouseShortcuts.onAction = { [weak self] action in
      // Keep the event-tap callback short; never run CLI or display UI in it.
      DispatchQueue.main.async {
        guard let self, self.mouseShortcutsEnabled else { return }
        switch action {
        case .toggleRecording: self.toggle()
        case .paste: self.pasteClipboard()
        }
      }
    }
    DispatchQueue.global(qos: .utility).async { _ = runCLI(["start"]) }
    refresh()
    if !preferences.bool(forKey: "hasLaunched") {
      preferences.set(true, forKey: "hasLaunched")
      showSettings()
    }
    timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
      self?.refresh()
    }
    RunLoop.main.add(timer!, forMode: .common)
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
      hotkeyError = "\(choice.name) 注册失败（\(code)），请选择其他快捷键"
    }
  }
  func refresh() {
    updateMouseShortcuts()
    var state: [String: Any] = [:]
    if let data = try? Data(contentsOf: stateURL()),
      let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    {
      state = value
    }
    let pid = state["pid"] as? Int32 ?? 0
    let running = pid > 0 && kill(pid, 0) == 0
    recording = running && state["recording"] as? Bool == true
    transcribing = running && state["last_event"] as? String == "transcribing"
    lastError = state["error"] as? String
    latestText = state["last_polished_text"] as? String ?? ""
    let event = state["last_event"] as? String ?? ""
    let eventID =
      "\(event)|\(state["audio_path"] as? String ?? "")|\(state["last_output_path"] as? String ?? "")|\(lastError ?? "")"
    if let previous = observedEvent, previous != eventID {
      switch event {
      case "recording_started":
        showHUD("开始录音", "再按 \(hotkeys[selected].name) 停止并转写", symbol: "mic.fill", persistent: true)
      case "transcribing":
        showHUD("已停止录音，正在转写", "正在识别语音并整理文字…", symbol: "waveform", persistent: true)
      case "pipeline_completed":
        if lastError != nil {
          showHUD("转写完成，但复制失败", "结果已保存，请从菜单重新复制或查看错误", symbol: "exclamationmark.triangle")
        } else {
          showHUD(
            "转写完成",
            state["clipboard_copied"] as? Bool == true ? "已复制，按鼠标下侧键或 ⌘V 粘贴" : "结果已保存，可从状态栏查看和复制",
            symbol: "checkmark.circle.fill")
        }
      case "recording_cancelled": showHUD("已取消录音", "本次音频未发送转写", symbol: "xmark.circle")
      case "pipeline_failed", "command_failed":
        showHUD("处理未完成", "请从状态栏查看错误详情后重试", symbol: "exclamationmark.triangle")
      default: hud?.orderOut(nil)
      }
    }
    observedEvent = eventID
    statusText =
      !running
      ? "后台服务未运行"
      : recording
        ? "正在录音，再按快捷键停止" : transcribing ? "正在转写和整理…" : lastError != nil ? "处理出错，点击查看详情" : "就绪"
    let fingerprint =
      "\(latestText)|\(statusText)|\(lastError ?? "")|\(selected)|\(hotkeyError ?? "")|\(loginEnabled)|\(busy)|\(settingsWindow != nil)|\(apiConfigured)|\(mouseShortcutStatus)|\(mouseShortcuts.swapped)"
    guard fingerprint != renderedState else { return }
    if !menuTracking { renderedState = fingerprint }
    item.button?.title = recording ? " ●" : transcribing ? " …" : ""
    item.button?.image = NSImage(
      systemSymbolName: recording ? "mic.fill" : "mic",
      accessibilityDescription: "OpenTyless：\(statusText)")
    item.button?.image?.isTemplate = true
    item.button?.toolTip =
      "OpenTyless：\(statusText)\n快捷键：\(hotkeys[selected].name)\n鼠标侧键：\(mouseShortcutStatus)"
      + (latestText.isEmpty ? "" : "\n最近结果：" + String(latestText.prefix(60)))
    let hint =
      apiConfigured
      ? "按快捷键或鼠标上侧键开始录音，再按一次停止并转写。\n转写完成后，按鼠标下侧键或 ⌘V 粘贴。\n侧键方向相反时，可从状态栏菜单交换上下侧键。"
      : "请先编辑 API 配置，填写 DASHSCOPE_API_KEY，\n再从状态栏选择「重新加载配置」。"
    settingsLabel?.stringValue =
      "后台：\(statusText)\n全局快捷键：\(hotkeys[selected].name)（\(hotkey == nil ? "未注册" : "已注册")）\n鼠标侧键：\(mouseShortcutStatus)\n登录自启动：\(loginEnabled ? "已开启" : "已关闭")\nAPI 配置：\(apiConfigured ? "已填写" : "待填写")\n\n"
      + hint
    if item.menu == nil || !menuTracking { buildMenu() }
  }
  func add(_ menu: NSMenu, _ title: String, _ action: Selector?, enabled: Bool = true) -> NSMenuItem
  {
    let entry = NSMenuItem(title: title, action: action, keyEquivalent: "")
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
    if let hotkeyError { _ = add(menu, hotkeyError, nil, enabled: false) }
    if lastError != nil { _ = add(menu, "查看错误详情…", #selector(showError)) }
    menu.addItem(.separator())
    _ = add(
      menu, recording ? "停止录音并转写" : "开始录音", #selector(toggle), enabled: !busy && !transcribing)
    if recording { _ = add(menu, "取消录音", #selector(cancel), enabled: !busy) }
    _ = add(menu, "查看最近结果…", #selector(showResult), enabled: !latestText.isEmpty)
    _ = add(menu, "复制最近结果", #selector(copyLast), enabled: !busy && !latestText.isEmpty)
    _ = add(menu, "打开输出目录", #selector(openOutputs))
    _ = add(menu, "显示状态提示", #selector(showStatus))
    _ = add(menu, "设置与状态…", #selector(showSettings))
    _ = add(menu, "编辑 API 配置…", #selector(openConfig))
    _ = add(menu, "检查麦克风权限…", #selector(microphone))
    let shortcut = add(menu, "全局快捷键：\(hotkeys[selected].name)", nil)
    let submenu = NSMenu()
    submenu.autoenablesItems = false
    for (index, choice) in hotkeys.enumerated() {
      let entry = add(submenu, choice.name, #selector(changeHotkey(_:)))
      entry.tag = index
      entry.state = selected == index && hotkey != nil ? .on : .off
    }
    shortcut.submenu = submenu
    let mouse = add(menu, "鼠标侧键：\(mouseShortcutStatus)", nil)
    let mouseMenu = NSMenu()
    mouseMenu.autoenablesItems = false
    let enabled = add(mouseMenu, "启用侧键：上侧录音，下侧粘贴", #selector(toggleMouseShortcuts))
    enabled.state = mouseShortcutsEnabled ? .on : .off
    let swap = add(mouseMenu, "交换上下侧键", #selector(swapMouseButtons))
    swap.state = mouseShortcuts.swapped ? .on : .off
    _ = add(mouseMenu, "授权辅助功能…", #selector(mousePermission))
    mouse.submenu = mouseMenu
    let login = add(
      menu, "登录时自动启动", #selector(toggleLogin), enabled: !busy && !recording && !transcribing)
    login.state = loginEnabled ? .on : .off
    _ = add(
      menu, "重启后台服务 / 重新加载配置", #selector(restart), enabled: !busy && !recording && !transcribing)
    _ = add(menu, "查看日志", #selector(openLogs))
    menu.addItem(.separator())
    _ = add(
      menu, "退出 OpenTyless（停止后台服务）", #selector(quit), enabled: !busy && !recording && !transcribing)
    item.menu = menu
  }
  func menuWillOpen(_ menu: NSMenu) { menuTracking = true }
  func menuDidClose(_ menu: NSMenu) { menuTracking = false }
  func updateMouseShortcuts() {
    guard mouseShortcutsEnabled && AXIsProcessTrusted() else {
      mouseShortcuts.stop()
      mouseShortcutError = nil
      return
    }
    if !mouseShortcuts.isRunning {
      mouseShortcutError = mouseShortcuts.start() ? nil : "监听失败，请检查辅助功能权限"
    }
  }
  @objc func toggleMouseShortcuts() {
    preferences.set(!mouseShortcutsEnabled, forKey: "mouseShortcutsEnabled")
    refresh()
    if mouseShortcutsEnabled && !AXIsProcessTrusted() { mousePermission() }
  }
  @objc func swapMouseButtons() {
    mouseShortcuts.swapped.toggle()
    preferences.set(mouseShortcuts.swapped, forKey: "mouseButtonsSwapped")
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
    // Paste the existing clipboard into the focused application, exactly as ⌘V.
    // Do not activate OpenTyless or replace the clipboard with an older result.
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
    busy = true
    DispatchQueue.global(qos: .userInitiated).async {
      let result = runCLI(["check-config"])
      DispatchQueue.main.async {
        self.busy = false
        guard result.0 == 0 else {
          self.alert(
            "请先配置 API Key", "在状态栏的「编辑 API 配置」中填写 DASHSCOPE_API_KEY，然后选择「重新加载配置」。\n\n" + result.1)
          return
        }
        if AVCaptureDevice.authorizationStatus(for: .audio) == .authorized {
          self.perform(["toggle-record"])
          return
        }
        self.busy = true
        AVCaptureDevice.requestAccess(for: .audio) { allowed in
          DispatchQueue.main.async {
            self.busy = false
            if allowed {
              self.perform(["toggle-record"])
            } else {
              self.alert("需要麦克风权限", "请在系统设置 → 隐私与安全性 → 麦克风中允许 OpenTyless。")
            }
          }
        }
      }
    }
  }
  func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool
  {
    showSettings()
    return true
  }
  @objc func showSettings() {
    if settingsWindow == nil {
      let window = NSWindow(
        contentRect: NSRect(x: 0, y: 0, width: 540, height: 340), styleMask: [.titled, .closable],
        backing: .buffered, defer: false)
      window.title = "OpenTyless · 设置与状态"
      window.isReleasedWhenClosed = false
      let label = NSTextField(wrappingLabelWithString: "")
      label.frame = NSRect(x: 24, y: 80, width: 492, height: 235)
      label.font = NSFont.systemFont(ofSize: 14)
      window.contentView?.addSubview(label)
      settingsLabel = label
      let config = NSButton(title: "编辑 API 配置", target: self, action: #selector(openConfig))
      config.frame = NSRect(x: 24, y: 24, width: 110, height: 32)
      config.bezelStyle = .rounded
      window.contentView?.addSubview(config)
      let permission = NSButton(title: "麦克风权限", target: self, action: #selector(microphone))
      permission.frame = NSRect(x: 150, y: 24, width: 110, height: 32)
      permission.bezelStyle = .rounded
      window.contentView?.addSubview(permission)
      let mouse = NSButton(title: "鼠标侧键权限", target: self, action: #selector(mousePermission))
      mouse.frame = NSRect(x: 276, y: 24, width: 110, height: 32)
      mouse.bezelStyle = .rounded
      window.contentView?.addSubview(mouse)
      let status = NSButton(title: "状态栏菜单", target: self, action: #selector(showMenu))
      status.frame = NSRect(x: 402, y: 24, width: 110, height: 32)
      status.bezelStyle = .rounded
      window.contentView?.addSubview(status)
      window.center()
      settingsWindow = window
    }
    refresh()
    NSApp.activate(ignoringOtherApps: true)
    settingsWindow?.makeKeyAndOrderFront(nil)
  }
  @objc func showMenu() {
    buildMenu()
    if let button = item.button, let menu = item.menu {
      menu.popUp(positioning: nil, at: NSPoint(x: 0, y: button.bounds.height), in: button)
    }
  }
  @objc func copyLast() {
    perform(["copy-last"]) { self.showHUD("复制成功", "按鼠标下侧键或 ⌘V 粘贴", symbol: "doc.on.clipboard") }
  }
  @objc func cancel() {
    // Cancel is exposed by the daemon socket; keep the same public CLI on both platforms.
    perform(["cancel-record"])
  }
  func showHUD(_ title: String, _ detail: String, symbol: String, persistent: Bool = false) {
    hudHide?.cancel()
    hud?.orderOut(nil)
    let panel = NSPanel(
      contentRect: NSRect(x: 0, y: 0, width: 360, height: 88),
      styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
    panel.title = "OpenTyless · 状态提示"
    panel.isOpaque = false
    panel.backgroundColor = .clear
    panel.hasShadow = true
    panel.level = .floating
    panel.ignoresMouseEvents = true
    panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
    let effect = NSVisualEffectView(frame: panel.contentView!.bounds)
    effect.material = .hudWindow
    effect.blendingMode = .behindWindow
    effect.state = .active
    effect.wantsLayer = true
    effect.layer?.cornerRadius = 16
    effect.layer?.masksToBounds = true
    panel.contentView = effect
    let icon = NSImageView(frame: NSRect(x: 20, y: 26, width: 30, height: 30))
    icon.image = NSImage(systemSymbolName: symbol, accessibilityDescription: title)
    icon.contentTintColor = .controlAccentColor
    effect.addSubview(icon)
    let heading = NSTextField(labelWithString: title)
    heading.font = .systemFont(ofSize: 15, weight: .semibold)
    heading.frame = NSRect(x: 64, y: 47, width: 280, height: 21)
    effect.addSubview(heading)
    let subtitle = NSTextField(labelWithString: detail)
    subtitle.font = .systemFont(ofSize: 12)
    subtitle.textColor = .secondaryLabelColor
    subtitle.frame = NSRect(x: 64, y: 22, width: 280, height: 20)
    effect.addSubview(subtitle)
    let screen =
      NSScreen.screens.first(where: { NSMouseInRect(NSEvent.mouseLocation, $0.frame, false) })
      ?? NSScreen.main
    if let screen {
      panel.setFrameOrigin(
        NSPoint(x: screen.visibleFrame.midX - 180, y: screen.visibleFrame.maxY - 110))
    }
    panel.orderFrontRegardless()
    hud = panel
    if !persistent {
      let work = DispatchWorkItem { [weak panel] in panel?.orderOut(nil) }
      hudHide = work
      DispatchQueue.main.asyncAfter(deadline: .now() + 4, execute: work)
    }
  }
  @objc func showResult() {
    if resultWindow == nil {
      let window = NSWindow(
        contentRect: NSRect(x: 0, y: 0, width: 560, height: 360),
        styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
      window.title = "OpenTyless · 最近结果"
      window.isReleasedWhenClosed = false
      window.minSize = NSSize(width: 400, height: 260)
      let scroll = NSScrollView(frame: NSRect(x: 20, y: 68, width: 520, height: 272))
      scroll.hasVerticalScroller = true
      scroll.autoresizingMask = [.width, .height]
      let text = NSTextView(frame: scroll.bounds)
      text.isEditable = false
      text.isSelectable = true
      text.isRichText = false
      text.font = .systemFont(ofSize: 15)
      text.textContainerInset = NSSize(width: 12, height: 12)
      text.isVerticallyResizable = true
      text.autoresizingMask = [.width]
      text.textContainer?.widthTracksTextView = true
      scroll.documentView = text
      window.contentView?.addSubview(scroll)
      resultText = text
      let copy = NSButton(title: "复制这段文字", target: self, action: #selector(copyPreview))
      copy.frame = NSRect(x: 20, y: 20, width: 140, height: 32)
      copy.bezelStyle = .rounded
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
    showHUD("复制成功", "按 ⌘V 粘贴这段文字", symbol: "doc.on.clipboard")
  }
  @objc func restart() { perform(["restart"]) }
  @objc func showStatus() {
    showHUD(
      "OpenTyless · " + statusText, "快捷键：" + hotkeys[selected].name,
      symbol: recording ? "mic.fill" : "mic")
  }
  @objc func showError() { alert("后台错误", lastError ?? "无错误") }
  @objc func changeHotkey(_ sender: NSMenuItem) {
    if sender.tag != selected || hotkey == nil { registerHotkey(sender.tag) }
    if let hotkeyError { alert("快捷键冲突", hotkeyError) }
    buildMenu()
  }
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
        self.alert("麦克风权限", allowed ? "已授权。" : "未授权，请在系统设置 → 隐私与安全性 → 麦克风中允许 OpenTyless。")
      }
    }
  }
  @objc func toggleLogin() {
    guard !busy else { return }
    let enabled = loginEnabled
    busy = true
    DispatchQueue.global(qos: .userInitiated).async {
      // Remove the tray plist directly: booting out our own launchd job
      // here would kill the menu before the operation completes.
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
        } else if enabled {
          // Disabling login startup should leave the current session usable.
          self.alert("已关闭登录自启动", "下次登录不会自动运行。当前会话仍可正常使用。")
        }
        self.refresh()
      }
    }
  }
  @objc func quit() { perform(["stop"]) { NSApp.terminate(nil) } }
  func applicationWillTerminate(_ notification: Notification) {
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
