// 実際の入力を送って操作を確かめる。使い方: poke <アプリ名> click <x> <y> | key <keycode> [cmd] [shift] | scroll <dy> <x> <y> | type <text>
import AppKit
import CoreGraphics
import Foundation
let a = CommandLine.arguments
// 入力は最前面のアプリに届く。利用者の別のアプリへ誤って送らないよう、対象が最前面のときだけ送る
guard NSWorkspace.shared.frontmostApplication?.localizedName == CommandLine.arguments[1] else {
    print("skip: \(CommandLine.arguments[1]) is not frontmost")
    exit(2)
}
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
guard let win = list.first(where: { ($0["kCGWindowOwnerName"] as? String) == a[1] }),
      let b = win["kCGWindowBounds"] as? [String: CGFloat] else { print("no window"); exit(1) }
let src = CGEventSource(stateID: .hidSystemState)
func at(_ x: String, _ y: String) -> CGPoint { CGPoint(x: b["X"]! + CGFloat(Double(x)!), y: b["Y"]! + CGFloat(Double(y)!)) }
switch a[2] {
case "click":
    let p = at(a[3], a[4])
    CGEvent(mouseEventSource: src, mouseType: .mouseMoved, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap)
    usleep(120000)
    CGEvent(mouseEventSource: src, mouseType: .leftMouseDown, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap)
    usleep(40000)
    CGEvent(mouseEventSource: src, mouseType: .leftMouseUp, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap)
case "cmdclick":
    let p = at(a[3], a[4])
    CGEvent(mouseEventSource: src, mouseType: .mouseMoved, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap)
    usleep(120000)
    for type in [CGEventType.leftMouseDown, .leftMouseUp] {
        let e = CGEvent(mouseEventSource: src, mouseType: type, mouseCursorPosition: p, mouseButton: .left)
        e?.flags = .maskCommand
        e?.post(tap: .cghidEventTap)
        usleep(40000)
    }
case "drag":
    let (from, to) = (at(a[3], a[4]), at(a[5], a[6]))
    CGEvent(mouseEventSource: src, mouseType: .mouseMoved, mouseCursorPosition: from, mouseButton: .left)?.post(tap: .cghidEventTap)
    usleep(120000)
    CGEvent(mouseEventSource: src, mouseType: .leftMouseDown, mouseCursorPosition: from, mouseButton: .left)?.post(tap: .cghidEventTap)
    usleep(60000)
    for i in 1...12 {
        let t = CGFloat(i) / 12
        let p = CGPoint(x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t)
        CGEvent(mouseEventSource: src, mouseType: .leftMouseDragged, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap)
        usleep(25000)
    }
    CGEvent(mouseEventSource: src, mouseType: .leftMouseUp, mouseCursorPosition: to, mouseButton: .left)?.post(tap: .cghidEventTap)
case "move":
    CGEvent(mouseEventSource: src, mouseType: .mouseMoved, mouseCursorPosition: at(a[3], a[4]), mouseButton: .left)?.post(tap: .cghidEventTap)
case "key":
    var flags: CGEventFlags = []
    if a.contains("cmd") { flags.insert(.maskCommand) }
    if a.contains("shift") { flags.insert(.maskShift) }
    for down in [true, false] {
        let e = CGEvent(keyboardEventSource: src, virtualKey: CGKeyCode(a[3])!, keyDown: down)
        e?.flags = flags
        e?.post(tap: .cghidEventTap)
        usleep(30000)
    }
case "scroll":
    CGEvent(mouseEventSource: src, mouseType: .mouseMoved, mouseCursorPosition: at(a[4], a[5]), mouseButton: .left)?.post(tap: .cghidEventTap)
    usleep(80000)
    CGEvent(scrollWheelEvent2Source: src, units: .pixel, wheelCount: 1, wheel1: Int32(a[3])!, wheel2: 0, wheel3: 0)?.post(tap: .cghidEventTap)
case "type":
    for ch in a[3].utf16 {
        for down in [true, false] {
            let e = CGEvent(keyboardEventSource: src, virtualKey: 0, keyDown: down)
            var c = ch
            e?.keyboardSetUnicodeString(stringLength: 1, unicodeString: &c)
            e?.post(tap: .cghidEventTap)
            usleep(20000)
        }
    }
default: break
}
