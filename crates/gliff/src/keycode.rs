//! The evdev code the server expects for a GDK hardware keycode.

/// On Linux the hardware keycode is the evdev code plus 8.
#[cfg(not(target_os = "macos"))]
pub fn evdev(keycode: u32) -> Option<u32> {
    Some(keycode.saturating_sub(8))
}

/// On macOS the hardware keycode is the virtual keycode (`kVK_*` in
/// HIToolbox's `Events.h`). Keys are mapped by position, except that
/// Command is Super and Option is Alt. Fn has no evdev key and is dropped.
#[cfg(target_os = "macos")]
pub fn evdev(keycode: u32) -> Option<u32> {
    Some(match keycode {
        0x00 => 30,  // A
        0x01 => 31,  // S
        0x02 => 32,  // D
        0x03 => 33,  // F
        0x04 => 35,  // H
        0x05 => 34,  // G
        0x06 => 44,  // Z
        0x07 => 45,  // X
        0x08 => 46,  // C
        0x09 => 47,  // V
        0x0A => 86,  // ISO section: KEY_102ND
        0x0B => 48,  // B
        0x0C => 16,  // Q
        0x0D => 17,  // W
        0x0E => 18,  // E
        0x0F => 19,  // R
        0x10 => 21,  // Y
        0x11 => 20,  // T
        0x12 => 2,   // 1
        0x13 => 3,   // 2
        0x14 => 4,   // 3
        0x15 => 5,   // 4
        0x16 => 7,   // 6
        0x17 => 6,   // 5
        0x18 => 13,  // =
        0x19 => 10,  // 9
        0x1A => 8,   // 7
        0x1B => 12,  // -
        0x1C => 9,   // 8
        0x1D => 11,  // 0
        0x1E => 27,  // ]
        0x1F => 24,  // O
        0x20 => 22,  // U
        0x21 => 26,  // [
        0x22 => 23,  // I
        0x23 => 25,  // P
        0x24 => 28,  // Return
        0x25 => 38,  // L
        0x26 => 36,  // J
        0x27 => 40,  // '
        0x28 => 37,  // K
        0x29 => 39,  // ;
        0x2A => 43,  // backslash
        0x2B => 51,  // ,
        0x2C => 53,  // /
        0x2D => 49,  // N
        0x2E => 50,  // M
        0x2F => 52,  // .
        0x30 => 15,  // Tab
        0x31 => 57,  // Space
        0x32 => 41,  // `
        0x33 => 14,  // Delete: KEY_BACKSPACE
        0x35 => 1,   // Escape
        0x36 => 126, // right Command: KEY_RIGHTMETA
        0x37 => 125, // Command: KEY_LEFTMETA
        0x38 => 42,  // Shift
        0x39 => 58,  // Caps Lock
        0x3A => 56,  // Option: KEY_LEFTALT
        0x3B => 29,  // Control
        0x3C => 54,  // right Shift
        0x3D => 100, // right Option: KEY_RIGHTALT
        0x3E => 97,  // right Control
        0x40 => 187, // F17
        0x41 => 83,  // keypad .
        0x43 => 55,  // keypad *
        0x45 => 78,  // keypad +
        0x47 => 69,  // keypad Clear: KEY_NUMLOCK
        0x48 => 115, // volume up
        0x49 => 114, // volume down
        0x4A => 113, // mute
        0x4B => 98,  // keypad /
        0x4C => 96,  // keypad Enter
        0x4E => 74,  // keypad -
        0x4F => 188, // F18
        0x50 => 189, // F19
        0x51 => 117, // keypad =
        0x52 => 82,  // keypad 0
        0x53 => 79,  // keypad 1
        0x54 => 80,  // keypad 2
        0x55 => 81,  // keypad 3
        0x56 => 75,  // keypad 4
        0x57 => 76,  // keypad 5
        0x58 => 77,  // keypad 6
        0x59 => 71,  // keypad 7
        0x5A => 190, // F20
        0x5B => 72,  // keypad 8
        0x5C => 73,  // keypad 9
        0x5D => 124, // JIS yen: KEY_YEN
        0x5E => 89,  // JIS underscore: KEY_RO
        0x5F => 95,  // JIS keypad comma: KEY_KPJPCOMMA
        0x60 => 63,  // F5
        0x61 => 64,  // F6
        0x62 => 65,  // F7
        0x63 => 61,  // F3
        0x64 => 66,  // F8
        0x65 => 67,  // F9
        0x66 => 123, // JIS Eisu: KEY_HANJA
        0x67 => 87,  // F11
        0x68 => 122, // JIS Kana: KEY_HANGEUL
        0x69 => 183, // F13
        0x6A => 186, // F16
        0x6B => 184, // F14
        0x6D => 68,  // F10
        0x6E => 127, // context menu: KEY_COMPOSE
        0x6F => 88,  // F12
        0x71 => 185, // F15
        0x72 => 110, // Help: KEY_INSERT
        0x73 => 102, // Home
        0x74 => 104, // Page Up
        0x75 => 111, // forward Delete: KEY_DELETE
        0x76 => 62,  // F4
        0x77 => 107, // End
        0x78 => 60,  // F2
        0x79 => 109, // Page Down
        0x7A => 59,  // F1
        0x7B => 105, // Left
        0x7C => 106, // Right
        0x7D => 108, // Down
        0x7E => 103, // Up
        _ => return None,
    })
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn modifiers_follow_their_names() {
        assert_eq!(evdev(0x37), Some(125)); // Command is Super
        assert_eq!(evdev(0x3A), Some(56)); // Option is Alt
        assert_eq!(evdev(0x3B), Some(29)); // Control
        assert_eq!(evdev(0x3F), None); // Fn
    }

    #[test]
    fn no_two_keys_share_a_code() {
        let mut seen = std::collections::HashMap::new();
        for k in 0..=0x7F {
            if let Some(code) = evdev(k) {
                assert!(seen.insert(code, k).is_none(), "evdev {code} used twice");
            }
        }
        assert!(seen.len() > 110);
    }
}
