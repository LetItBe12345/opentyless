import AppKit
import Carbon

@main
enum MouseShortcutsTests {
  static func main() {
    let shortcuts = MouseShortcuts()
    var recordings = 0
    var pastes = 0
    shortcuts.onAction = { action in
      switch action {
      case .toggleRecording: recordings += 1
      case .paste: pastes += 1
      }
    }
    func send(_ type: CGEventType, _ button: Int64) -> Bool {
      let event = CGEvent(
        mouseEventSource: nil, mouseType: .otherMouseDown, mouseCursorPosition: .zero,
        mouseButton: CGMouseButton(rawValue: UInt32(button))!)!
      return shortcuts.handle(type, event: event)
    }
    // A hold/repeat and drag must not stop a recording started by the press.
    precondition(send(.otherMouseDown, 4))
    precondition(send(.otherMouseDown, 4))
    precondition(send(.otherMouseDragged, 4))
    precondition(recordings == 1 && pastes == 0)
    precondition(send(.otherMouseUp, 4))
    precondition(recordings == 1)
    precondition(send(.otherMouseDown, 4))
    precondition(send(.otherMouseUp, 4))
    precondition(recordings == 2)

    precondition(send(.otherMouseDown, 3))
    precondition(send(.otherMouseUp, 3))
    precondition(pastes == 1 && recordings == 2)
    // Unassigned buttons and other event types must pass through untouched.
    for button: Int64 in [0, 1, 2, 5, 6] {
      precondition(!send(.otherMouseDown, button))
      precondition(!send(.otherMouseUp, button))
      precondition(!send(.otherMouseDragged, button))
    }
    precondition(!send(.keyDown, 4))
    precondition(pastes == 1 && recordings == 2)

    shortcuts.swapped = true
    precondition(send(.otherMouseDown, 3))
    precondition(send(.otherMouseUp, 3))
    precondition(send(.otherMouseDown, 4))
    precondition(send(.otherMouseUp, 4))
    precondition(recordings == 3 && pastes == 2)

    // Stopping must clear any held-button state before the next start.
    precondition(send(.otherMouseDown, 3))
    shortcuts.stop()
    precondition(send(.otherMouseDown, 3))
    precondition(send(.otherMouseUp, 3))
    precondition(recordings == 5)

    let events = MouseShortcuts.pasteEvents()
    precondition(events.count == 2)
    precondition(events[0].type == .keyDown && events[1].type == .keyUp)
    for event in events {
      precondition(event.getIntegerValueField(.keyboardEventKeycode) == Int64(kVK_ANSI_V))
      precondition(event.flags == .maskCommand)
    }
    print(
      "Passed: side-button clicks, holds, releases, drags, swapping, reset, and Command-V events")
  }
}
