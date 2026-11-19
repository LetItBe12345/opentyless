import AppKit
import ApplicationServices
import Carbon

enum MouseShortcutAction {
  case toggleRecording
  case paste
  case enter
}

// Only intercept the two thumb buttons. Leave the middle button and all other
// mouse/keyboard input alone, and consume releases/drags as well as presses so
// the frontmost application cannot also navigate back/forward.
final class MouseShortcuts {
  var swapped = false { didSet { resetPresses() } }
  var onAction: ((MouseShortcutAction) -> Void)?
  private var tap: CFMachPort?
  private var source: CFRunLoopSource?
  private var pressed: Set<Int64> = []
  private var pasteHoldTimer: Timer?
  private var pasteButton: Int64?
  private var enterSent = false
  var isRunning: Bool { tap.map { CGEvent.tapIsEnabled(tap: $0) } ?? false }

  @discardableResult func start() -> Bool {
    if let tap {
      CGEvent.tapEnable(tap: tap, enable: true)
      return true
    }
    guard AXIsProcessTrusted() else { return false }
    let mask = [CGEventType.otherMouseDown, .otherMouseUp, .otherMouseDragged]
      .reduce(CGEventMask(0)) { $0 | (CGEventMask(1) << $1.rawValue) }
    guard
      let tap = CGEvent.tapCreate(
        tap: .cgSessionEventTap, place: .headInsertEventTap, options: .defaultTap,
        eventsOfInterest: mask,
        callback: { _, type, event, pointer in
          guard let pointer else { return Unmanaged.passUnretained(event) }
          let shortcuts = Unmanaged<MouseShortcuts>.fromOpaque(pointer).takeUnretainedValue()
          if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            shortcuts.resetPresses()
            if let tap = shortcuts.tap { CGEvent.tapEnable(tap: tap, enable: true) }
            return Unmanaged.passUnretained(event)
          }
          return shortcuts.handle(type, event: event) ? nil : Unmanaged.passUnretained(event)
        }, userInfo: Unmanaged.passUnretained(self).toOpaque())
    else { return false }
    guard let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0) else {
      CFMachPortInvalidate(tap)
      return false
    }
    self.tap = tap
    self.source = source
    CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
    CGEvent.tapEnable(tap: tap, enable: true)
    return true
  }

  func stop() {
    if let tap {
      CGEvent.tapEnable(tap: tap, enable: false)
      CFMachPortInvalidate(tap)
    }
    if let source { CFRunLoopRemoveSource(CFRunLoopGetMain(), source, .commonModes) }
    source = nil
    tap = nil
    resetPresses()
  }

  private func resetPresses() {
    pasteHoldTimer?.invalidate()
    pasteHoldTimer = nil
    pasteButton = nil
    enterSent = false
    pressed.removeAll()
  }

  func handle(_ type: CGEventType, event: CGEvent) -> Bool {
    guard type == .otherMouseDown || type == .otherMouseUp || type == .otherMouseDragged
    else { return false }
    let button = event.getIntegerValueField(.mouseEventButtonNumber)
    guard button == 3 || button == 4 else { return false }
    if type == .otherMouseUp {
      let wasPressed = pressed.remove(button) != nil
      if wasPressed && button == pasteButton {
        pasteHoldTimer?.invalidate()
        pasteHoldTimer = nil
        pasteButton = nil
        if !enterSent { onAction?(.paste) }
      }
    } else if type == .otherMouseDown && pressed.insert(button).inserted {
      // Standard mice report forward/upper as button 4, back/lower as button 3
      // (zero based). Some devices reverse these, so persist a swap preference.
      if button == (swapped ? 3 : 4) {
        onAction?(.toggleRecording)
      } else {
        pasteButton = button
        enterSent = false
        let timer = Timer(timeInterval: 0.5, repeats: false) { [weak self] _ in
          guard let self else { return }
          self.enterSent = true
          self.pasteHoldTimer = nil
          self.onAction?(.enter)
        }
        pasteHoldTimer = timer
        RunLoop.main.add(timer, forMode: .common)
      }
    }
    return true
  }

  static func pasteEvents() -> [CGEvent] {
    keyEvents(CGKeyCode(kVK_ANSI_V), flags: .maskCommand)
  }

  static func enterEvents() -> [CGEvent] {
    keyEvents(CGKeyCode(kVK_Return), flags: [])
  }

  private static func keyEvents(_ key: CGKeyCode, flags: CGEventFlags) -> [CGEvent] {
    let source = CGEventSource(stateID: .privateState)
    guard
      let down = CGEvent(
        keyboardEventSource: source, virtualKey: key, keyDown: true),
      let up = CGEvent(
        keyboardEventSource: source, virtualKey: key, keyDown: false)
    else { return [] }
    // Do not inherit Option/Control/Shift currently held by the user.
    down.flags = flags
    up.flags = flags
    return [down, up]
  }

  deinit { stop() }
}
