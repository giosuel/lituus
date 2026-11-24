//! hyprpaper.conf (hyprpaper 0.8 and later): `wallpaper { monitor = …  path = …  fit_mode = … }`
//! blocks, which hyprpaper reads at start; what it's told while it runs isn't kept anywhere. Lituus
//! writes one block per monitor it sets, and keeps the rest of the file (splash, other blocks,
//! comments) as it is. A block whose path is a folder is hyprpaper's slideshow: a picture from it
//! every `timeout` seconds, in the folder's order or shuffled.

/// A `wallpaper` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Empty: hyprpaper's fallback for the monitors without one.
    pub monitor: String,
    pub path: String,
    /// "cover", "contain", "tile" or "fill" (stretched).
    pub fit_mode: String,
    /// A slideshow (`path` is a folder): seconds per picture; 0 for none.
    pub timeout: u32,
    /// The slideshow's order: "default" (the folder's) or "random-shuffle"; empty for none.
    pub order: String,
    /// The slideshow takes the pictures in the folder's subfolders too.
    pub recursive: bool,
}

impl Entry {
    /// One picture on a monitor.
    pub fn picture(monitor: &str, path: &str, fit_mode: &str) -> Entry {
        Entry {
            monitor: monitor.into(),
            path: path.into(),
            fit_mode: fit_mode.into(),
            timeout: 0,
            order: String::new(),
            recursive: false,
        }
    }
}

/// The fit mode as hyprpaper.conf writes it.
pub fn fit_mode(fill: crate::Fill) -> &'static str {
    match fill {
        crate::Fill::Cover | crate::Fill::Center => "cover",
        crate::Fill::Contain => "contain",
        crate::Fill::Stretch => "fill",
        crate::Fill::Tile => "tile",
    }
}

/// A line as hyprlang reads it: without its comment, which starts at a `#`, and with `##` as one
/// `#`; a line that starts with `#` is a comment whole.
fn uncommented(line: &str) -> String {
    if line.trim_start().starts_with('#') {
        return String::new();
    }
    let mut out = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '#' {
            out.push(c);
        } else if chars.as_str().starts_with('#') {
            chars.next();
            out.push('#');
            // hyprlang (0.6) takes the character after a `##` as it is, even another `#`.
            out.extend(chars.next());
        } else {
            break;
        }
    }
    out
}

/// A `wallpaper {` line.
fn opens(line: &str) -> bool {
    line.trim().strip_prefix("wallpaper").is_some_and(|rest| rest.trim_start().starts_with('{'))
}

/// A block's span in the file's lines: from its `wallpaper {` line to its `}`, and what it holds.
fn blocks(text: &str) -> Vec<(usize, usize, Entry)> {
    let lines: Vec<String> = text.lines().map(uncommented).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !opens(&lines[i]) {
            i += 1;
            continue;
        }
        let start = i;
        // hyprlang ends a block at a line that is `}` alone: a `}` in a value (a picture's path)
        // is part of it. A block on one line (`wallpaper { monitor = DP-1 }`), or one with the
        // `}` after its last value, ends at that `}`.
        i = if lines[i].trim_end().ends_with('}') {
            i
        } else {
            let rest: Vec<usize> = (i + 1..lines.len()).take_while(|j| !opens(&lines[*j])).collect();
            rest.iter()
                .find(|j| lines[**j].trim() == "}")
                .or_else(|| rest.iter().find(|j| lines[**j].trim_end().ends_with('}')))
                .or(rest.last())
                .copied()
                .unwrap_or(i)
        };
        let mut entry = Entry::picture("", "", "cover");
        for (at, line) in (start..=i).zip(&lines[start..=i]) {
            let mut field = if at == start { line.split_once('{').map_or("", |(_, b)| b) } else { line.as_str() };
            if at == i {
                field = field.trim_end().strip_suffix('}').unwrap_or(field);
            }
            if let Some((k, v)) = field.split_once('=') {
                let v = v.trim().to_owned();
                match k.trim() {
                    "monitor" => entry.monitor = v,
                    "path" => entry.path = v,
                    "fit_mode" => entry.fit_mode = v,
                    "timeout" => entry.timeout = v.parse().unwrap_or(0),
                    "order" => entry.order = v,
                    "recursive" => entry.recursive = matches!(v.as_str(), "1" | "true" | "yes" | "on"),
                    _ => {}
                }
            }
        }
        out.push((start, i, entry));
        i += 1;
    }
    out
}

/// Whether hyprpaper draws Hyprland's splash text over the picture (`splash`, off unless set).
pub fn splash(text: &str) -> bool {
    top_level(text)
        .into_iter()
        .find_map(|(_, k, v)| (k == "splash").then_some(matches!(v.as_str(), "1" | "true" | "yes" | "on")))
        .unwrap_or(false)
}

/// The file with `splash` set, the rest as it was (a new line goes at the top).
pub fn with_splash(text: &str, on: bool) -> String {
    let line = format!("splash = {on}");
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    match top_level(text).into_iter().find(|(_, k, _)| k == "splash") {
        Some((i, _, _)) => lines[i] = line,
        None => lines.insert(0, line),
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// The `key = value` lines outside the blocks: (line, key, value).
fn top_level(text: &str) -> Vec<(usize, String, String)> {
    let spans: Vec<(usize, usize)> = blocks(text).into_iter().map(|(start, end, _)| (start, end)).collect();
    text.lines()
        .enumerate()
        .filter(|(i, _)| !spans.iter().any(|(start, end)| (start..=end).contains(&i)))
        .filter_map(|(i, l)| {
            let (k, v) = uncommented(l).split_once('=').map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))?;
            Some((i, k, v))
        })
        .collect()
}

/// The `wallpaper` blocks in hyprpaper.conf's text.
pub fn entries(text: &str) -> Vec<Entry> {
    blocks(text).into_iter().map(|(_, _, e)| e).collect()
}

/// Why hyprlang can't read a path back as it is, if it can't. A `#` is written `##`, but hyprlang
/// (0.6) reads a run of them wrongly; `$` takes a variable's value (an environment variable's
/// too), `{{` starts a sum, a `\` can join lines or drop out (and hyprctl takes `\,` as an
/// escape), a line break ends the value, and a space at either end is trimmed.
pub fn unwritable(path: &str) -> Option<&'static str> {
    if path.contains("##") {
        Some("two # in a row")
    } else if path.contains('$') {
        Some("a $")
    } else if path.contains("{{") {
        Some("{{")
    } else if path.contains('\\') {
        Some("a \\")
    } else if path.contains(['\n', '\r']) {
        Some("a line break")
    } else if path.trim() != path {
        Some("a space at either end")
    } else {
        None
    }
}

/// A value as hyprlang reads it back: a lone `#` would start a comment.
fn escaped(value: &str) -> String {
    value.replace('#', "##")
}

/// The file with a block for each of `set` (replacing that monitor's, or added at the end), the
/// rest as it was. A picture whose path hyprlang can't hold (`unwritable`) isn't written: its
/// monitor keeps its block.
pub fn conf_text(text: &str, set: &[Entry]) -> String {
    let found = blocks(text);
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let set: Vec<&Entry> = set.iter().filter(|e| unwritable(&e.path).is_none()).collect();
    let block = |e: &Entry| {
        let mut b = format!(
            "wallpaper {{\n    monitor = {}\n    path = {}\n    fit_mode = {}\n",
            escaped(&e.monitor),
            escaped(&e.path),
            e.fit_mode
        );
        if e.timeout > 0 {
            b.push_str(&format!("    timeout = {}\n", e.timeout));
            if !e.order.is_empty() {
                b.push_str(&format!("    order = {}\n", e.order));
            }
            if e.recursive {
                b.push_str("    recursive = true\n");
            }
        }
        b.push('}');
        b
    };
    let mut added = Vec::new();
    // From the end, so the earlier spans stay where they are.
    let mut replaced: Vec<(usize, usize, String)> = Vec::new();
    for e in set {
        match found.iter().find(|(_, _, f)| f.monitor == e.monitor) {
            Some((start, end, _)) => replaced.push((*start, *end, block(e))),
            None => added.push(block(e)),
        }
    }
    replaced.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    for (start, end, text) in replaced {
        lines.splice(start..=end, text.lines().map(str::to_owned).collect::<Vec<_>>());
    }
    let mut out = lines.join("\n");
    if !added.is_empty() {
        if !out.trim().is_empty() {
            out = format!("{}\n\n", out.trim_end());
        }
        out.push_str(&added.join("\n\n"));
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_splash_line() {
        let text = "# mine\nwallpaper {\n    monitor = DP-1\n    splash = true\n}\n";
        assert!(!splash(text));
        let on = with_splash(text, true);
        assert_eq!(on, "splash = true\n# mine\nwallpaper {\n    monitor = DP-1\n    splash = true\n}\n");
        assert!(splash(&on));
        assert_eq!(with_splash(&on, false), on.replacen("splash = true", "splash = false", 1));
    }

    #[test]
    fn keeps_the_rest_of_the_file() {
        let text = "# My hyprpaper\nsplash = false\n\nwallpaper {\n    monitor = DP-3\n    path = ~/w/old.jpg # the old one\n}\n\nwallpaper { monitor = DP-4\n path = ~/w/b.png\n fit_mode = tile }\n";
        assert_eq!(
            entries(text),
            [Entry::picture("DP-3", "~/w/old.jpg", "cover"), Entry::picture("DP-4", "~/w/b.png", "tile"),]
        );
        let new = conf_text(
            text,
            &[Entry::picture("DP-3", "/w/alps.jpg", "fill"), Entry::picture("HDMI-A-3", "/w/lake.png", "cover")],
        );
        assert!(new.starts_with("# My hyprpaper\nsplash = false\n\nwallpaper {\n    monitor = DP-3\n    path = /w/alps.jpg\n    fit_mode = fill\n}\n"), "{new}");
        assert!(new.contains("wallpaper { monitor = DP-4\n path = ~/w/b.png\n fit_mode = tile }"), "{new}");
        assert!(
            new.ends_with("\n\nwallpaper {\n    monitor = HDMI-A-3\n    path = /w/lake.png\n    fit_mode = cover\n}\n"),
            "{new}"
        );
        assert_eq!(entries(&new).len(), 3);
        // A slideshow: the folder, a picture every 30 minutes, shuffled, with its subfolders.
        let show = Entry {
            timeout: 1800,
            order: "random-shuffle".into(),
            recursive: true,
            ..Entry::picture("DP-3", "/w", "cover")
        };
        let new = conf_text(&new, std::slice::from_ref(&show));
        assert!(new.contains("wallpaper {\n    monitor = DP-3\n    path = /w\n    fit_mode = cover\n    timeout = 1800\n    order = random-shuffle\n    recursive = true\n}\n"), "{new}");
        assert_eq!(entries(&new)[0], show);
        // And back to one picture.
        let new = conf_text(&new, &[Entry::picture("DP-3", "/w/a.jpg", "cover")]);
        assert!(!new.contains("timeout") && !new.contains("recursive"), "{new}");
        assert_eq!(
            conf_text("", &[Entry::picture("DP-1", "/a", "cover")]),
            "wallpaper {\n    monitor = DP-1\n    path = /a\n    fit_mode = cover\n}\n"
        );
    }

    #[test]
    fn a_hash_in_a_path_is_written_doubled() {
        // hyprlang ends a value at a lone `#` (a comment) and reads `##` as one `#`.
        let new = conf_text("", &[Entry::picture("DP-1", "/w/no#1.jpg", "cover")]);
        assert!(new.contains("    path = /w/no##1.jpg\n"), "{new}");
        assert_eq!(entries(&new), [Entry::picture("DP-1", "/w/no#1.jpg", "cover")]);
        // In the user's own lines too: `##` is a `#`, and a comment still ends the value.
        let mine = "wallpaper {\n    monitor = DP-1\n    path = /w/a##1.jpg # mine\n}\n";
        assert_eq!(entries(mine), [Entry::picture("DP-1", "/w/a#1.jpg", "cover")]);
        // hyprlang takes the character after `##` as it is: `####x` is `##`, then a comment.
        let run = "wallpaper {\n    monitor = DP-1\n    path = /w/two####x.jpg\n}\n";
        assert_eq!(entries(run)[0].path, "/w/two##");
        // A line that starts with `#` is a comment, `##` or not.
        assert!(entries("## wallpaper {\n").is_empty() && !splash("##splash = true\n"));
    }

    #[test]
    fn a_brace_in_a_path_stays_in_its_block() {
        let new = conf_text("", &[Entry::picture("DP-1", "/w/a}b.jpg", "tile")]);
        assert_eq!(entries(&new), [Entry::picture("DP-1", "/w/a}b.jpg", "tile")]);
        assert_eq!(
            conf_text(&new, &[Entry::picture("DP-1", "/w/c.jpg", "cover")]),
            "wallpaper {\n    monitor = DP-1\n    path = /w/c.jpg\n    fit_mode = cover\n}\n"
        );
        // A folder ending in `}`, and a splash line after it, which stays the file's own.
        let new = conf_text("", &[Entry::picture("DP-1", "/w/old}", "cover")]);
        let new = with_splash(&format!("{new}splash = true\n"), false);
        assert!(!splash(&new) && new.ends_with("}\nsplash = false\n"), "{new}");
        assert_eq!(entries(&new)[0].path, "/w/old}");
    }

    #[test]
    fn paths_hyprlang_changes_are_not_written() {
        for path in ["/w/a##b.jpg", "/w/$HOME.jpg", "/w/a{{b}}.jpg", "/w/back\\", "/w/a\nb.jpg", "/w/a.jpg "] {
            assert!(unwritable(path).is_some(), "{path}");
            let new = conf_text(
                "wallpaper {\n    monitor = DP-1\n    path = /w/old.jpg\n}\n",
                &[Entry::picture("DP-1", path, "cover")],
            );
            assert_eq!(entries(&new), [Entry::picture("DP-1", "/w/old.jpg", "cover")], "{path}");
        }
        for path in ["/w/#1 alps.jpg", "/w/a}b{c.jpg", "/w/Trip, 2024.jpg", "/w/it's.jpg"] {
            assert_eq!(unwritable(path), None, "{path}");
        }
    }
}
