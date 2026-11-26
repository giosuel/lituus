//! The user's own default printer, in ~/.cups/lpoptions (as `lpoptions -d` writes it): a
//! `Default <name>[/<instance>] [options]` line beats the system's default for this user only.
//! Lines for other printers (`Dest <name> options`) are kept.

use std::path::PathBuf;

pub fn path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cups/lpoptions"))
}

/// (keyword, the printer's name without its instance, the rest of the line)
fn split(line: &str) -> Option<(&str, &str, &str)> {
    let line = line.trim();
    let (kw, rest) = line.split_once(char::is_whitespace)?;
    if !kw.eq_ignore_ascii_case("default") && !kw.eq_ignore_ascii_case("dest") {
        return None;
    }
    let rest = rest.trim_start();
    let (dest, opts) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let name = dest.split('/').next().unwrap_or(dest);
    Some((kw, name, opts.trim()))
}

/// The printer the file makes the default.
pub fn default_in(text: &str) -> Option<String> {
    text.lines()
        .filter_map(split)
        .find(|(kw, ..)| kw.eq_ignore_ascii_case("default"))
        .map(|(_, name, _)| name.to_string())
}

/// The file with `name` as the default: its line becomes `Default`, the old default's a `Dest` line
/// (dropped when it had no options), everything else as it was.
pub fn with_default(text: &str, name: &str) -> String {
    let mut out = Vec::new();
    let mut found = false;
    for line in text.lines() {
        match split(line) {
            Some((_, n, opts)) if n == name => {
                found = true;
                let dest = line.split_whitespace().nth(1).unwrap_or(name);
                out.push(if opts.is_empty() { format!("Default {dest}") } else { format!("Default {dest} {opts}") });
            }
            Some((kw, _, opts)) if kw.eq_ignore_ascii_case("default") => {
                if !opts.is_empty() {
                    let dest = line.split_whitespace().nth(1).unwrap_or_default();
                    out.push(format!("Dest {dest} {opts}"));
                }
            }
            _ => out.push(line.to_string()),
        }
    }
    if !found {
        out.insert(0, format!("Default {name}"));
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

pub fn read_default() -> Option<String> {
    default_in(&std::fs::read_to_string(path()?).ok()?)
}

/// Makes `name` the user's default printer.
pub fn write_default(name: &str) -> std::io::Result<()> {
    let p = path().ok_or_else(|| std::io::Error::other("no home folder"))?;
    let old = std::fs::read_to_string(&p).unwrap_or_default();
    std::fs::create_dir_all(p.parent().expect("a file in .cups"))?;
    let tmp = p.with_extension("lituus-new");
    std::fs::write(&tmp, with_default(&old, name))?;
    std::fs::rename(&tmp, &p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_line_names_the_printer_without_its_instance() {
        assert_eq!(
            default_in("Dest Other media=A4\nDefault Office/duplex sides=two-sided-long-edge\n").as_deref(),
            Some("Office")
        );
        assert_eq!(default_in("Dest Other media=A4\n"), None);
        assert_eq!(default_in(""), None);
    }

    #[test]
    fn a_new_default_keeps_the_other_printers_lines_and_options() {
        assert_eq!(with_default("", "Office"), "Default Office\n");
        let old = "Default Home media=A4\nDest Office sides=one-sided\nDest Lab\n";
        assert_eq!(with_default(old, "Office"), "Dest Home media=A4\nDefault Office sides=one-sided\nDest Lab\n");
        assert_eq!(with_default("Default Home\nDest Lab media=A5\n", "Office"), "Default Office\nDest Lab media=A5\n");
        assert_eq!(with_default("Default Office\n", "Office"), "Default Office\n");
    }
}
