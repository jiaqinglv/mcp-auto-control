//! 设备报文的编解码。
//!
//! 报文格式由固件决定，改动前请先核对：
//! - 鼠标：`AutoMouse/src/mqtt.rs:140-182`
//! - 键盘：`AutoKeyboard/src/mqtt.rs:134-181`
//!
//! 两者都以 `msg_id: u64` 小端开头，固件仅用它打日志、不做去重。

/// 鼠标报文长度。**必须恒为 13**：固件用 `len >= 10` 校验却读取 `payload[12]`，
/// 发送 10~12 字节会触发切片越界 panic，进而停止喂看门狗导致设备复位。
pub const MOUSE_PAYLOAD_LEN: usize = 13;

/// 键盘报文长度。
pub const KEY_PAYLOAD_LEN: usize = 10;

pub const BUTTON_LEFT: u8 = 0x01;
pub const BUTTON_RIGHT: u8 = 0x02;
pub const BUTTON_MIDDLE: u8 = 0x04;

/// 鼠标报告：`[u64 msg_id][u8 buttons][i8 x][i8 y][i8 wheel][i8 pan]`。
///
/// `x` / `y` 是**相对增量**而非绝对坐标；`wheel` 正数向上滚，`pan` 正数向右滚。
pub fn encode_mouse(
    msg_id: u64,
    buttons: u8,
    x: i8,
    y: i8,
    wheel: i8,
    pan: i8,
) -> [u8; MOUSE_PAYLOAD_LEN] {
    let mut buf = [0u8; MOUSE_PAYLOAD_LEN];
    buf[0..8].copy_from_slice(&msg_id.to_le_bytes());
    buf[8] = buttons;
    buf[9] = x as u8;
    buf[10] = y as u8;
    buf[11] = wheel as u8;
    buf[12] = pan as u8;
    buf
}

/// 按键报告：`[u64 msg_id][u8 key_code][u8 pressed]`。
///
/// `key_code` 必须是 HID Usage ID，且必须落在固件 `KEY_MAPPING` 表中，否则固件会静默忽略。
pub fn encode_key(msg_id: u64, key_code: u8, pressed: bool) -> [u8; KEY_PAYLOAD_LEN] {
    let mut buf = [0u8; KEY_PAYLOAD_LEN];
    buf[0..8].copy_from_slice(&msg_id.to_le_bytes());
    buf[8] = key_code;
    buf[9] = u8::from(pressed);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_payload_layout_is_little_endian_13_bytes() {
        let p = encode_mouse(1, BUTTON_LEFT, 100, 50, -1, 2);
        assert_eq!(p.len(), 13);
        assert_eq!(&p[0..8], &[1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(p[8], 0x01);
        assert_eq!(p[9], 100);
        assert_eq!(p[10], 50);
        assert_eq!(p[11], 0xFF); // -1
        assert_eq!(p[12], 2);
    }

    #[test]
    fn mouse_payload_encodes_large_msg_id_little_endian() {
        let p = encode_mouse(0x0102_0304_0506_0708, 0, 0, 0, 0, 0);
        assert_eq!(&p[0..8], &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
    }

    /// 回归测试：任何调用路径都必须产出 13 字节，否则会打崩 AutoMouse 固件。
    #[test]
    fn mouse_payload_length_is_always_13() {
        for (x, y) in [(0i8, 0i8), (-128, 127), (127, -128), (-1, 1)] {
            assert_eq!(
                encode_mouse(u64::MAX, 0xFF, x, y, -128, 127).len(),
                MOUSE_PAYLOAD_LEN
            );
        }
    }

    #[test]
    fn mouse_i8_boundaries_round_trip() {
        let p = encode_mouse(0, 0, -128, 127, -128, 127);
        assert_eq!(p[9] as i8, -128);
        assert_eq!(p[10] as i8, 127);
        assert_eq!(p[11] as i8, -128);
        assert_eq!(p[12] as i8, 127);
    }

    #[test]
    fn key_payload_layout() {
        let down = encode_key(2, 0x04, true);
        assert_eq!(down.len(), 10);
        assert_eq!(&down[0..8], &[2, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(down[8], 0x04);
        assert_eq!(down[9], 1);

        let up = encode_key(3, 0x04, false);
        assert_eq!(up[9], 0);
    }

    #[test]
    fn button_masks_match_firmware_constants() {
        // AutoMouse/src/hid/mod.rs:75-79
        assert_eq!(BUTTON_LEFT | BUTTON_RIGHT | BUTTON_MIDDLE, 0x07);
    }
}
