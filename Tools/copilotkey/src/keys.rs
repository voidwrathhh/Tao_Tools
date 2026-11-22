//! 按键处理：名称解析、组合键格式化、修饰键状态、按键注入。

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN,
};

pub const MOD_WIN: u32 = 0b0001;
pub const MOD_SHIFT: u32 = 0b0010;
pub const MOD_CTRL: u32 = 0b0100;
pub const MOD_ALT: u32 = 0b1000;

/// 四个修饰键全部占用的位，比较时只取这 4 位。
pub const MOD_MASK: u32 = MOD_WIN | MOD_SHIFT | MOD_CTRL | MOD_ALT;

/// 微软规定 Copilot 键以 Win+Shift+F23 上报给系统，F23 = 0x86。
pub const VK_F23: u32 = 0x86;

/// 某个虚拟键此刻是否物理按下（`GetAsyncKeyState` 的 0x8000 位）。
fn is_down_now(vk: u32) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

/// 现算当前修饰键状态（位掩码）。
///
/// 判定瞬间调用，不依赖钩子累计的历史状态：UAC / 安全桌面期间低级钩子收不到键盘
/// 事件，累计状态会永久残留，导致“多按/少按修饰键”误判。
pub fn live_mods() -> u32 {
    let mut mods = 0u32;
    // 左右两侧都要看：0x5B/0x5C=Win，0xA0/0xA1=Shift，0xA2/0xA3=Ctrl，0xA4/0xA5=Alt。
    if is_down_now(0x5B) || is_down_now(0x5C) {
        mods |= MOD_WIN;
    }
    if is_down_now(0xA0) || is_down_now(0xA1) {
        mods |= MOD_SHIFT;
    }
    if is_down_now(0xA2) || is_down_now(0xA3) {
        mods |= MOD_CTRL;
    }
    if is_down_now(0xA4) || is_down_now(0xA5) {
        mods |= MOD_ALT;
    }
    mods
}

/// 修饰键是否满足触发条件。
///
/// - `strict = true`：修饰键必须**完全一致**，多按一个 Ctrl 也不触发；
/// - `strict = false`：只看主键，修饰键一概不管。
pub fn mods_match(strict: bool, live: u32, required: u32) -> bool {
    if !strict {
        return true;
    }
    (live & MOD_MASK) == (required & MOD_MASK)
}

/// 判断是不是修饰键，返回它对应的位。
pub fn modifier_bit(vk: u32) -> Option<u32> {
    let vk = vk as u16;
    if vk == VK_LWIN || vk == VK_RWIN {
        return Some(MOD_WIN);
    }
    if vk == VK_LSHIFT || vk == VK_RSHIFT {
        return Some(MOD_SHIFT);
    }
    if vk == VK_LCONTROL || vk == VK_RCONTROL {
        return Some(MOD_CTRL);
    }
    if vk == VK_LMENU || vk == VK_RMENU {
        return Some(MOD_ALT);
    }
    match vk {
        0x10 => Some(MOD_SHIFT),
        0x11 => Some(MOD_CTRL),
        0x12 => Some(MOD_ALT),
        0x5B | 0x5C => Some(MOD_WIN),
        _ => None,
    }
}

/// 解析单个按键名，例如 f23 / a / 5 / space / copilot。
pub fn parse_key(name: &str) -> Option<u32> {
    let name = name.trim().to_ascii_lowercase();
    let fixed: Option<u32> = match name.as_str() {
        "copilot" => Some(VK_F23),
        "win" | "lwin" | "super" | "meta" => Some(VK_LWIN as u32),
        "rwin" => Some(VK_RWIN as u32),
        "shift" => Some(VK_LSHIFT as u32),
        "ctrl" | "control" | "ctl" => Some(VK_LCONTROL as u32),
        "alt" | "menu" => Some(VK_LMENU as u32),
        "space" => Some(0x20),
        "tab" => Some(0x09),
        "enter" | "return" => Some(0x0D),
        "esc" | "escape" => Some(0x1B),
        "backspace" | "bs" => Some(0x08),
        "insert" | "ins" => Some(0x2D),
        "delete" | "del" => Some(0x2E),
        "home" => Some(0x24),
        "end" => Some(0x23),
        "pageup" | "pgup" => Some(0x21),
        "pagedown" | "pgdn" => Some(0x22),
        "up" => Some(0x26),
        "down" => Some(0x28),
        "left" => Some(0x25),
        "right" => Some(0x27),
        "capslock" | "caps" => Some(0x14),
        "apps" | "contextmenu" => Some(0x5D),
        "printscreen" | "prtsc" => Some(0x2C),
        "pause" => Some(0x13),
        "scrolllock" => Some(0x91),
        "numlock" => Some(0x90),
        "-" | "minus" => Some(0xBD),
        "=" | "plus" => Some(0xBB),
        "[" => Some(0xDB),
        "]" => Some(0xDD),
        ";" => Some(0xBA),
        "'" => Some(0xDE),
        "," => Some(0xBC),
        "." => Some(0xBE),
        "/" => Some(0xBF),
        "\\" => Some(0xDC),
        "`" => Some(0xC0),
        _ => None,
    };
    if fixed.is_some() {
        return fixed;
    }
    if let Some(rest) = name.strip_prefix('f') {
        if let Ok(num) = rest.parse::<u32>() {
            if (1..=24).contains(&num) {
                return Some(0x70 + num - 1);
            }
        }
    }
    if name.len() == 1 {
        let ch = name.as_bytes()[0];
        if ch.is_ascii_alphabetic() {
            return Some(ch.to_ascii_uppercase() as u32);
        }
        if ch.is_ascii_digit() {
            return Some(ch as u32);
        }
    }
    None
}

/// 虚拟键码转可读名称。
pub fn key_name(vk: u32) -> String {
    let name: Option<String> = match vk {
        0x70..=0x87 => Some(format!("F{}", vk - 0x70 + 1)),
        0x41..=0x5A => Some(((vk as u8) as char).to_string()),
        0x30..=0x39 => Some(((vk as u8) as char).to_string()),
        0x5B => Some("Win".to_string()),
        0x5C => Some("RWin".to_string()),
        0x10 | 0xA0 | 0xA1 => Some("Shift".to_string()),
        0x11 | 0xA2 | 0xA3 => Some("Ctrl".to_string()),
        0x12 | 0xA4 | 0xA5 => Some("Alt".to_string()),
        0x20 => Some("Space".to_string()),
        0x09 => Some("Tab".to_string()),
        0x0D => Some("Enter".to_string()),
        0x1B => Some("Esc".to_string()),
        0x08 => Some("Backspace".to_string()),
        0x2D => Some("Insert".to_string()),
        0x2E => Some("Delete".to_string()),
        0x24 => Some("Home".to_string()),
        0x23 => Some("End".to_string()),
        0x21 => Some("PageUp".to_string()),
        0x22 => Some("PageDown".to_string()),
        0x26 => Some("Up".to_string()),
        0x28 => Some("Down".to_string()),
        0x25 => Some("Left".to_string()),
        0x27 => Some("Right".to_string()),
        0x2C => Some("PrintScreen".to_string()),
        0x13 => Some("Pause".to_string()),
        _ => None,
    };
    name.unwrap_or_else(|| format!("VK_0x{:02X}", vk))
}

/// 修饰键位掩码转可读名称，例如 "Win+Shift"。
pub fn mods_label(mods: u32) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if mods & MOD_WIN != 0 {
        parts.push("Win");
    }
    if mods & MOD_SHIFT != 0 {
        parts.push("Shift");
    }
    if mods & MOD_CTRL != 0 {
        parts.push("Ctrl");
    }
    if mods & MOD_ALT != 0 {
        parts.push("Alt");
    }
    parts.join("+")
}

/// 组合键可读名称，例如 "Win+Shift+F23"。
pub fn combo_label(vk: u32, mods: u32) -> String {
    let first = mods_label(mods);
    if first.is_empty() {
        key_name(vk)
    } else {
        format!("{}+{}", first, key_name(vk))
    }
}

/// 解析 "win+shift+f23" 这类组合键，返回 (虚拟键码, 修饰键掩码)。
pub fn parse_combo(text: &str) -> Result<(u32, u32), String> {
    let parts: Vec<String> = text
        .split('+')
        .map(|p| p.trim().to_ascii_lowercase())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Err("组合键为空，例如 win+shift+f23".to_string());
    }
    let (last, heads) = parts.split_last().unwrap();
    let mut mods = 0u32;
    for head in heads {
        let bit = match head.as_str() {
            "win" | "lwin" | "super" | "meta" => MOD_WIN,
            "shift" => MOD_SHIFT,
            "ctrl" | "control" | "ctl" => MOD_CTRL,
            "alt" | "menu" => MOD_ALT,
            other => return Err(format!("不认识的修饰键：{}", other)),
        };
        mods |= bit;
    }
    let vk = parse_key(last).ok_or_else(|| format!("不认识的按键：{}", last))?;
    if modifier_bit(vk).is_some() {
        return Err(format!(
            "“{}” 是修饰键，还需要一个主键，例如 win+shift+f23",
            last
        ));
    }
    Ok((vk, mods))
}

fn make_input(vk: u16, keyup: bool) -> INPUT {
    let mut input: INPUT = unsafe { std::mem::zeroed() };
    input.r#type = INPUT_KEYBOARD;
    input.Anonymous = INPUT_0 {
        ki: KEYBDINPUT {
            wVk: vk,
            wScan: 0,
            dwFlags: if keyup { KEYEVENTF_KEYUP } else { 0 },
            time: 0,
            dwExtraInfo: 0,
        },
    };
    input
}

/// 模拟按下一次组合键（用于 test 子命令做端到端验证）。
pub fn send_combo(vk: u32, mods: u32) {
    let virtual_mods: [u16; 4] = [
        VK_LWIN,
        VK_LSHIFT,
        VK_LCONTROL,
        VK_LMENU,
    ];
    let bits = [MOD_WIN, MOD_SHIFT, MOD_CTRL, MOD_ALT];

    let mut sequence: Vec<(u16, bool)> = Vec::new();
    for (idx, vk_mod) in virtual_mods.iter().enumerate() {
        if mods & bits[idx] != 0 {
            sequence.push((*vk_mod, false));
        }
    }
    sequence.push((vk as u16, false));
    sequence.push((vk as u16, true));
    for (idx, vk_mod) in virtual_mods.iter().enumerate().rev() {
        if mods & bits[idx] != 0 {
            sequence.push((*vk_mod, true));
        }
    }

    let inputs: Vec<INPUT> = sequence
        .iter()
        .map(|(code, keyup)| make_input(*code, *keyup))
        .collect();
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loose_mode_only_cares_about_main_key() {
        // strict_mods=false：修饰键多按、少按、不按都算命中。
        assert!(mods_match(false, 0, 0b0011));
        assert!(mods_match(false, 0b0111, 0b0011));
        assert!(mods_match(false, 0, 0));
    }

    #[test]
    fn strict_mode_requires_exact_modifier_match() {
        // 完全一致 → 触发。
        assert!(mods_match(true, 0b0011, 0b0011));
        assert!(mods_match(true, 0, 0));
        // 多按一个 Ctrl → 不触发（v1.0.1 的“只验要求的都在”会误触发）。
        assert!(!mods_match(true, 0b0111, 0b0011));
        // 少按 Shift → 不触发。
        assert!(!mods_match(true, 0b0001, 0b0011));
        // Alt 之类的多余修饰键同样不触发。
        assert!(!mods_match(true, 0b1011, 0b0011));
    }

    #[test]
    fn strict_comparison_ignores_bits_outside_modifier_mask() {
        assert!(mods_match(true, 0b0011 | 0x8000_0000, 0b0011));
    }

    #[test]
    fn live_mods_never_reports_bits_outside_modifier_mask() {
        // 不假设当前是否按着键，只验证返回值不越界（真机按 F23 的链路无法自动验证）。
        let mods = live_mods();
        assert_eq!(mods & !MOD_MASK, 0);
    }
}
