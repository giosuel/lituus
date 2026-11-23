//! The One UI library's check (D-46): sizes of type and colours come from theme.slint's tokens
//! (`Type.*`, `Theme.*`), not from numbers written into a page. A raw `font-size: 14px` or a
//! `#rrggbb` anywhere but in theme.slint (and the icon and logo paths) fails here, with where.

use std::path::Path;

fn ui_files() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "slint"))
        .filter(|p| !["theme.slint", "icons.slint", "logo.slint"].contains(&p.file_name().unwrap().to_str().unwrap()))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    out.sort();
    out
}

#[test]
fn no_raw_font_sizes() {
    let mut found = Vec::new();
    for (name, text) in ui_files() {
        for (i, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or_default();
            if let Some(at) = code.find("font-size:") {
                let value = code[at + "font-size:".len()..].trim_start();
                if value.starts_with(|c: char| c.is_ascii_digit()) {
                    found.push(format!("{name}:{}: {}", i + 1, line.trim()));
                }
            }
        }
    }
    assert!(found.is_empty(), "use a Type.* size from theme.slint:\n{}", found.join("\n"));
}

#[test]
fn no_raw_colours() {
    let mut found = Vec::new();
    for (name, text) in ui_files() {
        for (i, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or_default();
            let bytes = code.as_bytes();
            for (at, _) in code.match_indices('#') {
                let hex = bytes[at + 1..].iter().take_while(|b| b.is_ascii_hexdigit()).count();
                if hex == 3 || hex == 6 || hex == 8 {
                    found.push(format!("{name}:{}: {}", i + 1, line.trim()));
                    break;
                }
            }
        }
    }
    assert!(found.is_empty(), "use a Theme.* colour from theme.slint:\n{}", found.join("\n"));
}
