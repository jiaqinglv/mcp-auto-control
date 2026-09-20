//! 键盘动作语义：键名映射、字符映射、组合键与文本输入。
//!
//! **键位白名单来自固件**：AutoKeyboard 收到 HID KeyCode 后先查自己的
//! `KEY_MAPPING` 表得到矩阵坐标，查不到就打印 `Invalid key code` 并丢弃
//! （`AutoKeyboard/src/mqtt.rs:153-168`）。因此服务端只允许下发该表内的键，
//! 其余一律报错，绝不静默下发。
//!
//! 各键的数值取自固件实际使用的 rmk 版本（git checkout `4bbcd54`）的
//! `rmk-types/src/keycode/hid.rs`，不是按 HID 规范推测的。

use anyhow::{Result, bail};

use super::{Devices, Report};

/// 单次文本输入的字符数上限。
const MAX_TEXT_CHARS: usize = 512;

pub const KEY_LEFT_CTRL: u8 = 0xE0;
pub const KEY_LEFT_SHIFT: u8 = 0xE1;
pub const KEY_LEFT_ALT: u8 = 0xE2;
pub const KEY_LEFT_GUI: u8 = 0xE3;
pub const KEY_RIGHT_CTRL: u8 = 0xE4;
pub const KEY_RIGHT_SHIFT: u8 = 0xE5;
pub const KEY_RIGHT_ALT: u8 = 0xE6;

/// 键名 → HID KeyCode。与固件 `KEY_MAPPING`（`AutoKeyboard/src/keymap.rs:15-446`）一一对应。
const SUPPORTED_KEYS: &[(&str, u8)] = &[
    // 字母
    ("a", 0x04),
    ("b", 0x05),
    ("c", 0x06),
    ("d", 0x07),
    ("e", 0x08),
    ("f", 0x09),
    ("g", 0x0A),
    ("h", 0x0B),
    ("i", 0x0C),
    ("j", 0x0D),
    ("k", 0x0E),
    ("l", 0x0F),
    ("m", 0x10),
    ("n", 0x11),
    ("o", 0x12),
    ("p", 0x13),
    ("q", 0x14),
    ("r", 0x15),
    ("s", 0x16),
    ("t", 0x17),
    ("u", 0x18),
    ("v", 0x19),
    ("w", 0x1A),
    ("x", 0x1B),
    ("y", 0x1C),
    ("z", 0x1D),
    // 数字行（按字符命名）
    ("1", 0x1E),
    ("2", 0x1F),
    ("3", 0x20),
    ("4", 0x21),
    ("5", 0x22),
    ("6", 0x23),
    ("7", 0x24),
    ("8", 0x25),
    ("9", 0x26),
    ("0", 0x27),
    // 主键区
    ("enter", 0x28),
    ("esc", 0x29),
    ("backspace", 0x2A),
    ("tab", 0x2B),
    ("space", 0x2C),
    ("minus", 0x2D),
    ("equal", 0x2E),
    ("leftbracket", 0x2F),
    ("rightbracket", 0x30),
    ("backslash", 0x31),
    ("nonushash", 0x32),
    ("semicolon", 0x33),
    ("quote", 0x34),
    ("grave", 0x35),
    ("comma", 0x36),
    ("dot", 0x37),
    ("slash", 0x38),
    // 功能键
    ("f1", 0x3A),
    ("f2", 0x3B),
    ("f3", 0x3C),
    ("f4", 0x3D),
    ("f5", 0x3E),
    ("f6", 0x3F),
    ("f7", 0x40),
    ("f8", 0x41),
    ("f9", 0x42),
    ("f10", 0x43),
    ("f11", 0x44),
    ("f12", 0x45),
    // 编辑与方向
    ("printscreen", 0x46),
    ("scrolllock", 0x47),
    ("pause", 0x48),
    ("insert", 0x49),
    ("home", 0x4A),
    ("pageup", 0x4B),
    ("delete", 0x4C),
    ("end", 0x4D),
    ("pagedown", 0x4E),
    ("right", 0x4F),
    ("left", 0x50),
    ("down", 0x51),
    ("up", 0x52),
    // 修饰键（无 RGui：固件 KEY_MAPPING 中不存在）
    ("ctrl", KEY_LEFT_CTRL),
    ("leftctrl", KEY_LEFT_CTRL),
    ("shift", KEY_LEFT_SHIFT),
    ("leftshift", KEY_LEFT_SHIFT),
    ("alt", KEY_LEFT_ALT),
    ("leftalt", KEY_LEFT_ALT),
    ("gui", KEY_LEFT_GUI),
    ("leftgui", KEY_LEFT_GUI),
    ("win", KEY_LEFT_GUI),
    ("meta", KEY_LEFT_GUI),
    ("rightctrl", KEY_RIGHT_CTRL),
    ("rightshift", KEY_RIGHT_SHIFT),
    ("rightalt", KEY_RIGHT_ALT),
];

/// 键名归一化：小写并去掉分隔符，使 `Page_Up`、`page-up`、`pageup` 等价。
fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '_' && *c != '-' && !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 是否是修饰键。
pub fn is_modifier(key_code: u8) -> bool {
    (KEY_LEFT_CTRL..=KEY_RIGHT_ALT).contains(&key_code)
}

/// 查键名对应的 HID KeyCode；不在固件白名单内时返回 `None`。
pub fn lookup_key(name: &str) -> Option<u8> {
    let key = normalize(name);
    SUPPORTED_KEYS
        .iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, usage)| *usage)
}

/// 可用键名列表，用于错误提示。修饰键的别名只列出主名。
pub fn supported_key_names() -> Vec<&'static str> {
    SUPPORTED_KEYS.iter().map(|(name, _)| *name).collect()
}

/// 单个字符 → (HID KeyCode, 是否需要按住 Shift)。
///
/// 非 ASCII 字符一律返回 `None`：设备只有 US ANSI 键位表，没有 Unicode 注入通道。
pub fn char_to_keystroke(c: char) -> Option<(u8, bool)> {
    let pair = match c {
        'a'..='z' => ((c as u8 - b'a') + 0x04, false),
        'A'..='Z' => ((c as u8 - b'A') + 0x04, true),
        '1'..='9' => (0x1E + (c as u8 - b'1'), false),
        '0' => (0x27, false),
        ' ' => (0x2C, false),
        '\n' => (0x28, false),
        '\t' => (0x2B, false),
        // 符号：未按 Shift 与按住 Shift 是同一个物理键
        '-' => (0x2D, false),
        '_' => (0x2D, true),
        '=' => (0x2E, false),
        '+' => (0x2E, true),
        '[' => (0x2F, false),
        '{' => (0x2F, true),
        ']' => (0x30, false),
        '}' => (0x30, true),
        '\\' => (0x31, false),
        '|' => (0x31, true),
        ';' => (0x33, false),
        ':' => (0x33, true),
        '\'' => (0x34, false),
        '"' => (0x34, true),
        '`' => (0x35, false),
        '~' => (0x35, true),
        ',' => (0x36, false),
        '<' => (0x36, true),
        '.' => (0x37, false),
        '>' => (0x37, true),
        '/' => (0x38, false),
        '?' => (0x38, true),
        '!' => (0x1E, true),
        '@' => (0x1F, true),
        '#' => (0x20, true),
        '$' => (0x21, true),
        '%' => (0x22, true),
        '^' => (0x23, true),
        '&' => (0x24, true),
        '*' => (0x25, true),
        '(' => (0x26, true),
        ')' => (0x27, true),
        _ => return None,
    };
    Some(pair)
}

/// 把一个键名解析为 KeyCode，失败时给出可用键提示。
fn require_key(name: &str) -> Result<u8> {
    lookup_key(name).ok_or_else(|| {
        anyhow::anyhow!(
            "不支持的键名 \"{name}\"：设备固件只接受 KEY_MAPPING 中的键。\
             可用键名见 `keyboard_key` 工具说明（如 a-z、0-9、enter、esc、tab、space、\
             backspace、delete、up/down/left/right、home/end、pageup/pagedown、insert、\
             f1-f12、ctrl、shift、alt、gui、right_ctrl/right_shift/right_alt）"
        )
    })
}

impl Devices {
    /// 按下并保持某个键。会记录到已按下集合，供 `keyboard_release_all` 释放。
    pub async fn key_press(&self, key: &str) -> Result<Report> {
        let usage = require_key(key)?;
        self.check_budget(1)?;

        let mut sequence = self.begin().await?;
        if sequence.is_key_down(usage).await {
            bail!("键 \"{key}\" 已经是按下状态，重复按下会被忽略");
        }
        sequence.key_frame(usage, true).await?;
        Ok(sequence.finish())
    }

    /// 松开某个键。
    pub async fn key_release(&self, key: &str) -> Result<Report> {
        let usage = require_key(key)?;
        self.check_budget(1)?;

        let mut sequence = self.begin().await?;
        if !sequence.is_key_down(usage).await {
            bail!("键 \"{key}\" 当前并未被本服务按下，无需松开");
        }
        sequence.key_frame(usage, false).await?;
        Ok(sequence.finish())
    }

    /// 按一下再松开。
    pub async fn key_tap(&self, key: &str) -> Result<Report> {
        let usage = require_key(key)?;
        self.check_budget(2)?;

        let mut sequence = self.begin().await?;
        sequence.key_frame(usage, true).await?;
        sequence.key_frame(usage, false).await?;
        Ok(sequence.finish())
    }

    /// 组合键：前面的键按住，最后一个键点击，最后统一释放。
    ///
    /// 例如 `["ctrl", "shift", "s"]` 等价于 Ctrl+Shift+S。
    pub async fn key_combo(&self, keys: &[String]) -> Result<Report> {
        if keys.is_empty() {
            bail!("keys 不能为空");
        }
        let codes = keys
            .iter()
            .map(|name| require_key(name).map(|code| (name.clone(), code)))
            .collect::<Result<Vec<_>>>()?;

        let (_, main) = codes.last().expect("keys 非空已在上方校验").clone();
        // 组合键以「全部按下 + 全部松开」表达，最坏 2 * len 帧
        self.check_budget(codes.len() * 2)?;

        let mut sequence = self.begin().await?;
        let mut held = Vec::with_capacity(codes.len());
        for (name, code) in &codes {
            if *code == main {
                continue;
            }
            if sequence.is_key_down(*code).await {
                // 已经按住（例如上一次调用遗留），不重复按下，也不在结尾释放它
                tracing::debug!(key = %name, "修饰键已处于按下状态，跳过多余的按下帧");
                continue;
            }
            sequence.key_frame(*code, true).await?;
            held.push(*code);
        }
        sequence.key_frame(main, true).await?;
        sequence.key_frame(main, false).await?;

        // 逆序释放本服务按下的修饰键
        for code in held.into_iter().rev() {
            sequence.key_frame(code, false).await?;
        }
        Ok(sequence.finish())
    }

    /// 逐字符输入文本。仅支持 ASCII；非 ASCII 字符会直接报错（不静默跳过）。
    pub async fn type_text(&self, text: &str) -> Result<Report> {
        if text.is_empty() {
            bail!("text 不能为空");
        }
        let chars: Vec<char> = text.chars().collect();
        if chars.len() > MAX_TEXT_CHARS {
            bail!("text 长度 {} 超过上限 {MAX_TEXT_CHARS}", chars.len());
        }

        let mut plan = Vec::with_capacity(chars.len());
        for (index, c) in chars.iter().enumerate() {
            let keystroke = char_to_keystroke(*c).ok_or_else(|| {
                anyhow::anyhow!(
                    "无法输入第 {} 个字符 {:?}（U+{:04X}）：设备是 US ANSI 键位、无输入法，\
                     只支持 ASCII 可见字符与 \\n \\t",
                    index + 1,
                    c,
                    *c as u32
                )
            })?;
            plan.push(keystroke);
        }

        // 最坏情况每字符 3 帧（按 Shift + 按键 + 放 Shift），再加最后释放 Shift
        self.check_budget(plan.len() * 3 + 1)?;

        let mut sequence = self.begin().await?;
        let mut shift_held = sequence.is_key_down(KEY_LEFT_SHIFT).await;
        for (usage, need_shift) in plan {
            if need_shift != shift_held {
                sequence.key_frame(KEY_LEFT_SHIFT, need_shift).await?;
                shift_held = need_shift;
            }
            sequence.key_frame(usage, true).await?;
            sequence.key_frame(usage, false).await?;
        }
        if shift_held {
            sequence.key_frame(KEY_LEFT_SHIFT, false).await?;
        }
        Ok(sequence.finish())
    }

    /// 释放本服务按下过的所有键。
    pub async fn release_all_keys(&self) -> Result<Report> {
        self.release_pressed_keys().await
    }

    /// 急停：释放所有键 + 所有鼠标按钮。
    ///
    /// 只解除**本服务**按下的状态；物理按键不受影响（见 `doc/方案设计.md` R12）。
    pub async fn emergency_stop(&self) -> Result<Report> {
        let pressed = self.state().await.keys.len();
        self.check_budget(pressed + 1)?;

        let mut sequence = self.begin().await?;
        for code in sequence.keys_down().await {
            sequence.key_frame(code, false).await?;
        }
        if sequence.buttons().await != 0 {
            sequence.mouse_frame(0, 0, 0, 0, 0).await?;
        }
        Ok(sequence.finish())
    }

    async fn release_pressed_keys(&self) -> Result<Report> {
        let pressed = self.state().await.keys.len();
        self.check_budget(pressed + 1)?;

        let mut sequence = self.begin().await?;
        for code in sequence.keys_down().await {
            sequence.key_frame(code, false).await?;
        }
        Ok(sequence.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_key_is_unique_and_in_hid_range() {
        let mut names: Vec<&str> = supported_key_names();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "SUPPORTED_KEYS 中存在重复键名");

        for (name, usage) in SUPPORTED_KEYS {
            assert!(
                (0x04..=0xE7).contains(usage),
                "{name} 的 KeyCode {usage:#04X} 不在 HID 键盘页范围内"
            );
        }
    }

    #[test]
    fn supported_keys_exclude_what_firmware_lacks() {
        // 固件 KEY_MAPPING 中不存在这些键，必须查不到
        for missing in [
            "capslock", "caps", "rgui", "rightgui", "numpad1", "volumeup", "app",
        ] {
            assert!(lookup_key(missing).is_none(), "{missing} 不应被接受");
        }
    }

    #[test]
    fn key_names_are_normalized() {
        assert_eq!(lookup_key("Page_Up"), lookup_key("pageup"));
        assert_eq!(lookup_key("page-up"), lookup_key("PAGEUP"));
        assert_eq!(lookup_key("Left_Ctrl"), Some(KEY_LEFT_CTRL));
        assert_eq!(lookup_key("ESC"), Some(0x29));
        assert_eq!(lookup_key("ctrl"), Some(KEY_LEFT_CTRL));
        assert_eq!(lookup_key("right_alt"), Some(KEY_RIGHT_ALT));
    }

    #[test]
    fn modifiers_are_detected() {
        assert!(is_modifier(KEY_LEFT_CTRL));
        assert!(is_modifier(KEY_RIGHT_ALT));
        assert!(!is_modifier(0x04));
        assert!(!is_modifier(0x29));
    }

    #[test]
    fn ascii_letters_and_digits() {
        assert_eq!(char_to_keystroke('a'), Some((0x04, false)));
        assert_eq!(char_to_keystroke('A'), Some((0x04, true)));
        assert_eq!(char_to_keystroke('z'), Some((0x1D, false)));
        assert_eq!(char_to_keystroke('1'), Some((0x1E, false)));
        assert_eq!(char_to_keystroke('0'), Some((0x27, false)));
        assert_eq!(char_to_keystroke(' '), Some((0x2C, false)));
    }

    #[test]
    fn shifted_symbols_share_the_physical_key() {
        for (plain, shifted, usage) in [
            ('-', '_', 0x2D),
            ('=', '+', 0x2E),
            ('[', '{', 0x2F),
            (']', '}', 0x30),
            ('\\', '|', 0x31),
            (';', ':', 0x33),
            ('\'', '"', 0x34),
            ('`', '~', 0x35),
            (',', '<', 0x36),
            ('.', '>', 0x37),
            ('/', '?', 0x38),
        ] {
            assert_eq!(char_to_keystroke(plain), Some((usage, false)), "{plain}");
            assert_eq!(char_to_keystroke(shifted), Some((usage, true)), "{shifted}");
        }
        for (shifted, usage) in [
            ('!', 0x1E),
            ('@', 0x1F),
            ('#', 0x20),
            ('$', 0x21),
            ('%', 0x22),
            ('^', 0x23),
            ('&', 0x24),
            ('*', 0x25),
            ('(', 0x26),
            (')', 0x27),
        ] {
            assert_eq!(char_to_keystroke(shifted), Some((usage, true)), "{shifted}");
        }
    }

    #[test]
    fn non_ascii_is_rejected() {
        assert!(char_to_keystroke('中').is_none());
        assert!(char_to_keystroke('é').is_none());
        assert!(char_to_keystroke('😀').is_none());
    }

    #[test]
    fn text_characters_map_to_whitelisted_keys() {
        // 文本输入的每个字符都必须落在固件白名单内，否则设备会静默忽略
        let whitelist: Vec<u8> = SUPPORTED_KEYS.iter().map(|(_, usage)| *usage).collect();
        for c in "Hello, World! 123 @#$%\n\t".chars() {
            let (usage, _) = char_to_keystroke(c).expect("测试文本应全部可映射");
            assert!(
                whitelist.contains(&usage),
                "字符 {c:?} 的 KeyCode {usage:#04X} 不在白名单内"
            );
        }
    }

    #[test]
    fn shift_key_is_whitelisted() {
        assert!(lookup_key("shift") == Some(KEY_LEFT_SHIFT));
    }
}
