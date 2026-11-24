//! The Exec key, as the Desktop Entry spec reads and writes it. The value's string escapes
//! (`\s`, `\\`, …) come off first; then an argument with a space or another reserved character
//! is in double quotes, where `"`, `` ` ``, `$` and `\` take a backslash. `%` starts a field code
//! (`%f`, `%u`, …), and `%%` is a `%`. So `Exec="/opt/My Apps/a \\$b" %U` runs `/opt/My Apps/a $b`.

use std::ops::Range;
use std::path::Path;

/// One argument of a command: its text (unquoted, field codes left in), and the bytes of the
/// Exec value it was read from, quotes included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    pub span: Range<usize>,
}

/// The value without its string escapes: each character, with where it starts in `raw`. An
/// unknown escape keeps its backslash, as most launchers read it.
fn unescape(raw: &str) -> Vec<(char, usize)> {
    let mut out = Vec::new();
    let mut chars = raw.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '\\' {
            out.push((c, i));
            continue;
        }
        let plain = match chars.peek().map(|&(_, n)| n) {
            Some('s') => ' ',
            Some('n') => '\n',
            Some('t') => '\t',
            Some('r') => '\r',
            Some('\\') => '\\',
            _ => {
                out.push((c, i));
                continue;
            }
        };
        chars.next();
        out.push((plain, i));
    }
    out
}

/// A command's arguments, as a launcher splits it.
pub fn words(exec: &str) -> Vec<Word> {
    let chars = unescape(exec);
    let mut out = Vec::new();
    let mut word: Option<(String, usize)> = None;
    let mut k = 0;
    while k < chars.len() {
        let (c, at) = chars[k];
        k += 1;
        if matches!(c, ' ' | '\t' | '\n') {
            if let Some((text, start)) = word.take() {
                out.push(Word { text, span: start..at });
            }
            continue;
        }
        let (text, _) = word.get_or_insert_with(|| (String::new(), at));
        match c {
            // In double quotes, a backslash escapes only these four.
            '"' => {
                while k < chars.len() && chars[k].0 != '"' {
                    let (q, _) = chars[k];
                    if q == '\\' && chars.get(k + 1).is_some_and(|&(n, _)| matches!(n, '"' | '`' | '$' | '\\')) {
                        k += 1;
                    }
                    text.push(chars[k].0);
                    k += 1;
                }
                k += 1;
            }
            // Not in the spec, but GLib's parser (a shell's) takes them: everything up to the next '.
            '\'' => {
                while k < chars.len() && chars[k].0 != '\'' {
                    text.push(chars[k].0);
                    k += 1;
                }
                k += 1;
            }
            '\\' if k < chars.len() => {
                text.push(chars[k].0);
                k += 1;
            }
            c => text.push(c),
        }
    }
    // The last word runs to the end (a quote left open, too).
    if let Some((text, start)) = word {
        out.push(Word { text, span: start..exec.len() });
    }
    out
}

/// The spec's reserved characters: an argument with one of them is quoted.
const RESERVED: [char; 19] =
    [' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')', '`'];

/// An argument as an Exec value writes it: `%` as `%%`, in double quotes when it has a reserved
/// character, and the value's string escapes on top (a backslash written twice).
pub fn quote(arg: &str) -> String {
    let arg = arg.replace('%', "%%");
    let quoted = if arg.is_empty() || arg.contains(RESERVED) {
        let mut q = String::from('"');
        for c in arg.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                q.push('\\');
            }
            q.push(c);
        }
        q.push('"');
        q
    } else {
        arg
    };
    let mut out = String::with_capacity(quoted.len());
    for c in quoted.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

/// The field codes that stand for files and links (and the deprecated ones): a launcher that
/// opens none drops them.
const FILE_CODES: [char; 10] = ['f', 'F', 'u', 'U', 'd', 'D', 'n', 'N', 'v', 'm'];

/// What a launcher runs for an app it opens with no file: the command's arguments with `%i` as
/// `--icon <icon>`, `%c` as the app's name, `%k` as its desktop file, `%%` as `%`, and the other
/// field codes gone.
pub fn argv(exec: &str, name: &str, icon: &str, file: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for w in words(exec) {
        match w.text.as_str() {
            "%i" => {
                if !icon.is_empty() {
                    out.extend(["--icon".to_owned(), icon.to_owned()]);
                }
                continue;
            }
            t if t.len() == 2 && t.starts_with('%') && t.ends_with(FILE_CODES) => continue,
            _ => {}
        }
        let mut arg = String::new();
        let mut chars = w.text.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                arg.push(c);
                continue;
            }
            match chars.next() {
                Some('%') => arg.push('%'),
                Some('c') => arg.push_str(name),
                Some('k') => arg.push_str(&file.display().to_string()),
                _ => {}
            }
        }
        out.push(arg);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(exec: &str) -> Vec<String> {
        words(exec).into_iter().map(|w| w.text).collect()
    }

    #[test]
    fn quoting_and_escapes() {
        assert_eq!(split("firefox --new-window %u"), ["firefox", "--new-window", "%u"]);
        assert_eq!(split("\"/opt/My Apps/tool\"  %U"), ["/opt/My Apps/tool", "%U"]);
        // In the file, a quoted `$` or `"` takes two backslashes: the string's, then the quote's.
        assert_eq!(split(r#""/a/b \\"c\\" \\$d" x"#), ["/a/b \"c\" $d", "x"]);
        // `\s` is a plain space by the time the quotes are read, so it separates.
        assert_eq!(split(r"/opt/My\sApp/run"), ["/opt/My", "App/run"]);
        assert_eq!(split("env A=\"x y\" app 'single quoted'"), ["env", "A=x y", "app", "single quoted"]);
        let w = words("app \"a  b\" %U");
        assert_eq!((w[1].text.as_str(), w[1].span.clone()), ("a  b", 4..10));
    }

    #[test]
    fn what_quote_writes_reads_back() {
        assert_eq!(quote("/usr/bin/chromium"), "/usr/bin/chromium");
        assert_eq!(quote("/home/u/My Apps/a.sh"), "\"/home/u/My Apps/a.sh\"");
        assert_eq!(quote("/a/my$tool"), r#""/a/my\\$tool""#);
        assert_eq!(quote("--app=https://x.com/a%20b?q=1&r=2"), "\"--app=https://x.com/a%%20b?q=1&r=2\"");
        assert_eq!(quote(""), "\"\"");
        for arg in ["plain", "a b", "a\"b`c$d\\e", "x;y&z", "tab\there", "50%", "~/x"] {
            let exec = format!("{} {}", quote("/bin/p"), quote(arg));
            let read = argv(&exec, "", "", Path::new("/a.desktop"));
            assert_eq!(read, ["/bin/p", arg], "{exec}");
        }
    }

    #[test]
    fn the_field_codes_for_a_launch() {
        let file = Path::new("/usr/share/applications/a.desktop");
        assert_eq!(argv("app %U", "App", "app", file), ["app"]);
        assert_eq!(
            argv("app %i --name=%c %k", "App", "app-icon", file),
            ["app", "--icon", "app-icon", "--name=App", "/usr/share/applications/a.desktop"]
        );
        assert_eq!(argv("app %i 100%% \"%f\"", "App", "", file), ["app", "100%"]);
    }
}
