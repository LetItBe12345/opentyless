import AppKit
import ApplicationServices
import Carbon

enum MouseShortcutAction {
  case toggleRecording
  case paste
}

// Only intercept the two thumb buttons. Leave the middle button and all other
// mouse/keyboard input alone, and consume releases/drags as well as presses so
// the frontmost application cannot also navigate back/forward.
final class MouseShortcuts {
  var swapped = false
  var onAction: ((MouseShortcutAction) -> Void)?
  private var tap: CFMachPort?
  private var source: CFRunLoopSource?
  private var pressed: Set<Int64> = []
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
            shortcuts.pressed.removeAll()
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
    pressed.removeAll()
  }

  func handle(_ type: CGEventType, event: CGEvent) -> Bool {
    guard type == .otherMouseDown || type == .otherMouseUp || type == .otherMouseDragged
    else { return false }
    let button = event.getIntegerValueField(.mouseEventButtonNumber)
    guard button == 3 || button == 4 else { return false }
    if type == .otherMouseUp {
      pressed.remove(button)
    } else if type == .otherMouseDown && pressed.insert(button).inserted {
      // Standard mice report forward/upper as button 4, back/lower as button 3
      // (zero based). Some devices reverse these, so persist a swap preference.
      onAction?(button == (swapped ? 3 : 4) ? .toggleRecording : .paste)
    }
    return true
  }

  static func pasteEvents() -> [CGEvent] {
    let source = CGEventSource(stateID: .privateState)
    guard
      let down = CGEvent(
        keyboardEventSource: source, virtualKey: CGKeyCode(kVK_ANSI_V), keyDown: true),
      let up = CGEvent(
        keyboardEventSource: source, virtualKey: CGKeyCode(kVK_ANSI_V), keyDown: false)
    else { return [] }
    // Do not inherit Option/Control/Shift currently held by the user.
    down.flags = .maskCommand
    up.flags = .maskCommand
    return [down, up]
  }

  deinit { stop() }
}
