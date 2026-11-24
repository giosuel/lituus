//! The keyboard layout in use, compiled with libxkbcommon: which key types a character. Hyprland
//! matches a shortcut by its key's symbol without Shift, so that's what a recorded shortcut says:
//! Shift+1 types "+" on a Swiss keyboard and is SHIFT + 1, "ä" is "adiaeresis", and on a French
//! one Shift+& types "1" and is SHIFT + ampersand.

use std::collections::HashMap;

use xkbcommon::xkb;

/// Each symbol the layout types, and the symbol of the key that types it, without Shift.
#[derive(Debug, Default)]
pub struct Keymap {
    base: HashMap<u32, u32>,
}

impl Keymap {
    /// A layout and its variant ("ch", ""), with xkb's options; none when xkeyboard-config
    /// doesn't know them.
    pub fn new(layout: &str, variant: &str, options: &str) -> Option<Keymap> {
        if [layout, variant, options].iter().any(|s| s.contains('\0')) {
            return None;
        }
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(
            &context,
            "evdev",
            "pc105",
            layout,
            variant,
            Some(options.to_owned()),
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )?;
        // Each symbol goes to the key that types it best: a key of the main block before the
        // others (xkeyboard-config's inet(evdev) gives "(", ")", "$" and "€" multimedia keys of
        // their own, which nobody presses for them), then the lowest level (a key's own symbol
        // without Shift is its own, even if another key types it with Shift), then the first.
        let mut best: HashMap<u32, ((bool, u32, u32), u32)> = HashMap::new();
        for raw in keymap.min_keycode().raw()..=keymap.max_keycode().raw() {
            let code = xkb::Keycode::new(raw);
            let levels = keymap.num_levels_for_key(code, 0);
            let syms: Vec<Vec<u32>> = (0..levels)
                .map(|level| keymap.key_get_syms_by_level(code, 0, level).iter().map(|s| s.raw()).collect())
                .collect();
            let Some(&first) = syms.first().and_then(|s| s.first()) else { continue };
            for (level, sym) in syms.iter().enumerate().flat_map(|(l, s)| s.iter().map(move |s| (l as u32, *s))) {
                let rank = (!main_block(raw), level, raw);
                if best.get(&sym).is_none_or(|(r, _)| rank < *r) {
                    best.insert(sym, (rank, first));
                }
            }
        }
        Some(Keymap { base: best.into_iter().map(|(sym, (_, key))| (sym, key)).collect() })
    }

    /// The layout Hyprland's main keyboard uses now: one of its `kb_layout` ("ch,us") with its
    /// variant.
    pub fn of(keyboard: &lituus_hypr::Keyboard) -> Option<Keymap> {
        let index = keyboard.active_layout_index.unwrap_or(0);
        let layout = keyboard.layout.split(',').nth(index).unwrap_or_default().trim();
        let variant = keyboard.variant.split(',').nth(index).unwrap_or_default().trim();
        Keymap::new(if layout.is_empty() { "us" } else { layout }, variant, &keyboard.options)
    }

    /// The name of the key that types `c`, as Hyprland names keys: "1" for "+" on a Swiss
    /// keyboard. None for a character the layout doesn't type.
    pub fn key_name(&self, c: char) -> Option<String> {
        let sym = xkb::utf32_to_keysym(c as u32);
        let base = *self.base.get(&sym.raw())?;
        Some(xkb::keysym_get_name(xkb::Keysym::new(base)))
    }
}

/// A key of the keyboard's main block (xkb's keycodes are evdev's plus 8): Esc to F12 with the
/// keypad and the ISO key beside Shift, and the Japanese and Brazilian keys that type `\` and `/`.
fn main_block(code: u32) -> bool {
    matches!(code, 9..=97 | 132)
}

/// The character a key's name types ("adiaeresis" ä, "ampersand" &), for its key cap; none for
/// keys without one (F5, Print) and the keypad's.
pub fn character(name: &str) -> Option<char> {
    if name.starts_with("KP_") || name.contains('\0') {
        return None;
    }
    let mut sym = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
    if sym.raw() == 0 {
        sym = xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE);
    }
    char::from_u32(xkb::keysym_to_utf32(sym)).filter(|c| !c.is_control() && !c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_by_the_layout() {
        let ch = Keymap::new("ch", "", "").expect("xkeyboard-config's Swiss layout");
        assert_eq!(ch.key_name('+').as_deref(), Some("1"));
        assert_eq!(ch.key_name('ä').as_deref(), Some("adiaeresis"));
        // On the ö key with Shift.
        assert_eq!(ch.key_name('é').as_deref(), Some("odiaeresis"));
        assert_eq!(ch.key_name('z').as_deref(), Some("z"));
        assert_eq!(ch.key_name('Z').as_deref(), Some("z"));
        assert_eq!(ch.key_name('<').as_deref(), Some("less"));
        let fr = Keymap::new("fr", "", "").unwrap();
        assert_eq!(fr.key_name('1').as_deref(), Some("ampersand"));
        let us = Keymap::new("us", "", "").unwrap();
        assert_eq!(us.key_name('!').as_deref(), Some("1"));
        assert_eq!(us.key_name('ä'), None);
        assert!(Keymap::new("no-such-layout", "", "").is_none());
    }

    /// The multimedia keys "(", ")", "$" and "€" of xkeyboard-config's inet(evdev) aren't the
    /// keys a keyboard types those with; Hyprland matches the key pressed.
    #[test]
    fn symbols_typed_with_shift_or_altgr_are_their_main_keys() {
        let us = Keymap::new("us", "", "").unwrap();
        assert_eq!(us.key_name('(').as_deref(), Some("9"));
        assert_eq!(us.key_name(')').as_deref(), Some("0"));
        assert_eq!(us.key_name('$').as_deref(), Some("4"));
        let ch = Keymap::new("ch", "", "").unwrap();
        assert_eq!(ch.key_name('(').as_deref(), Some("8"));
        assert_eq!(ch.key_name('€').as_deref(), Some("e"));
    }

    #[test]
    fn characters_of_key_names() {
        assert_eq!(character("adiaeresis"), Some('ä'));
        assert_eq!(character("ampersand"), Some('&'));
        assert_eq!(character("F5"), None);
        assert_eq!(character("Return"), None);
        assert_eq!(character("KP_Add"), None);
        assert_eq!(character("code:10"), None);
    }

    #[test]
    fn the_layout_in_use() {
        let keyboard = lituus_hypr::Keyboard {
            layout: "us,ch".into(),
            variant: ",".into(),
            active_layout_index: Some(1),
            ..Default::default()
        };
        assert_eq!(Keymap::of(&keyboard).unwrap().key_name('+').as_deref(), Some("1"));
    }
}
