//! Соответствие кодов клавиш Linux (evdev, `KEY_*` из `linux/input-event-codes.h`) и
//! виртуальных кодов Windows (`VK_*`), которыми пользуются ядро и горячие клавиши.

/// (код evdev, VK)
#[rustfmt::skip]
const TABLE: &[(u16, u16)] = &[
    (1, 0x1B),                                     // Esc
    (2, 0x31), (3, 0x32), (4, 0x33), (5, 0x34), (6, 0x35),
    (7, 0x36), (8, 0x37), (9, 0x38), (10, 0x39), (11, 0x30),
    (12, 0xBD), (13, 0xBB), (14, 0x08), (15, 0x09), // - = Backspace Tab
    (16, 0x51), (17, 0x57), (18, 0x45), (19, 0x52), (20, 0x54), // Q W E R T
    (21, 0x59), (22, 0x55), (23, 0x49), (24, 0x4F), (25, 0x50), // Y U I O P
    (26, 0xDB), (27, 0xDD), (28, 0x0D), (29, 0xA2), // [ ] Enter LCtrl
    (30, 0x41), (31, 0x53), (32, 0x44), (33, 0x46), (34, 0x47), // A S D F G
    (35, 0x48), (36, 0x4A), (37, 0x4B), (38, 0x4C),             // H J K L
    (39, 0xBA), (40, 0xDE), (41, 0xC0), (42, 0xA0), (43, 0xDC), // ; ' ` LShift \
    (44, 0x5A), (45, 0x58), (46, 0x43), (47, 0x56), (48, 0x42), // Z X C V B
    (49, 0x4E), (50, 0x4D), (51, 0xBC), (52, 0xBE), (53, 0xBF), // N M , . /
    (54, 0xA1), (56, 0xA4), (57, 0x20), (58, 0x14), // RShift LAlt Space CapsLock
    (59, 0x70), (60, 0x71), (61, 0x72), (62, 0x73), (63, 0x74), // F1–F5
    (64, 0x75), (65, 0x76), (66, 0x77), (67, 0x78), (68, 0x79), // F6–F10
    (69, 0x90), (70, 0x91), (86, 0xE2), (87, 0x7A), (88, 0x7B), // NumLock ScrollLock 102nd F11 F12
    (96, 0x0D), (97, 0xA3), (99, 0x2C), (100, 0xA5),            // KPEnter RCtrl PrintScreen RAlt
    (102, 0x24), (103, 0x26), (104, 0x21), (105, 0x25), (106, 0x27), // Home Up PgUp Left Right
    (107, 0x23), (108, 0x28), (109, 0x22), (110, 0x2D), (111, 0x2E), // End Down PgDn Insert Delete
    (119, 0x13), (125, 0x5B), (126, 0x5C), (127, 0x5D),        // Pause LMeta RMeta Menu
    (183, 0x7C), (184, 0x7D), (185, 0x7E), (186, 0x7F), (187, 0x80), (188, 0x81), // F13–F18
    (189, 0x82), (190, 0x83), (191, 0x84), (192, 0x85), (193, 0x86), (194, 0x87), // F19–F24
];

/// VK по коду evdev. Неизвестные клавиши получают VK 0xFF — ядро считает их «сбросом».
pub fn vk_of(code: u16) -> u16 {
    TABLE.iter().find(|(c, _)| *c == code).map_or(0xFF, |(_, vk)| *vk)
}

/// Код evdev по VK (для печати). Для Enter — основной, не цифровой клавиатуры.
pub fn code_of(vk: u16) -> Option<u16> {
    TABLE.iter().find(|(_, v)| *v == vk).map(|(c, _)| *c)
}

/// Все коды, которые умеет нажимать виртуальная клавиатура.
pub fn all_codes() -> impl Iterator<Item = u16> {
    TABLE.iter().map(|(c, _)| *c)
}

pub const KEY_LEFTSHIFT: u16 = 42;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_ENTER: u16 = 28;
pub const KEY_CAPSLOCK: u16 = 58;
pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_LEFT: u16 = 105;
pub const KEY_A: u16 = 30;
pub const KEY_SPACE: u16 = 57;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_and_back() {
        assert_eq!(vk_of(30), 0x41); // A
        assert_eq!(vk_of(16), 0x51); // Q
        assert_eq!(code_of(0x51), Some(16));
        assert_eq!(code_of(0x0D), Some(KEY_ENTER));
        assert_eq!(vk_of(9999), 0xFF);
    }

    #[test]
    fn every_layout_key_is_typeable() {
        // Каждая клавиша, у которой есть символ в раскладках, должна печататься.
        for vk in (0x30..=0x39)
            .chain(0x41..=0x5A)
            .chain([0x20, 0xBA, 0xBB, 0xBC, 0xBD, 0xBE, 0xBF, 0xC0, 0xDB, 0xDC, 0xDD, 0xDE])
        {
            assert!(code_of(vk).is_some(), "нет кода для VK {vk:#x}");
        }
    }
}
