//! hypridle's configuration (~/.config/hypr/hypridle.conf): what happens after how long idle.
//! Anyone's file is read, by what its listeners run; only a file Lituus wrote is written again.
//! Taking over someone else's (the dotfiles') is the page's to ask, and keeps a .bak.

/// The first line of a file Lituus wrote.
pub const HEADER: &str = "# Written by Lituus (Power). Lituus rewrites this file; your own copy is hypridle.conf.bak.";

/// What hypridle does when idle. Times in seconds; `None` is never.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Idle {
    pub dim: Option<u32>,
    pub lock: Option<u32>,
    pub screens_off: Option<u32>,
    pub suspend: Option<u32>,
    /// Lock before the computer sleeps (before_sleep_cmd).
    pub lock_before_sleep: bool,
    /// The lock screen's program: "hyprlock", "swaylock", "gtklock".
    pub locker: String,
    /// The file is Lituus's own: it can be written.
    pub ours: bool,
}

/// The lock screens Lituus knows how to start, best first.
pub const LOCKERS: [&str; 3] = ["hyprlock", "swaylock", "gtklock"];

/// One `name { key = value … }` block of the file.
#[derive(Debug, Default)]
struct Block {
    name: String,
    keys: Vec<(String, String)>,
}

impl Block {
    fn get(&self, key: &str) -> Option<&str> {
        self.keys.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// The top-level blocks, with their keys (a nested block's keys belong to it, and are dropped).
fn blocks(text: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut depth = 0usize;
    for raw in text.lines() {
        // `##` is hyprlang's escaped `#`.
        let line = strip_comment(raw);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_suffix('{') {
            depth += 1;
            if depth == 1 {
                out.push(Block { name: name.trim().to_owned(), ..Block::default() });
            }
            continue;
        }
        if line == "}" {
            depth = depth.saturating_sub(1);
            continue;
        }
        if depth == 1
            && let Some((k, v)) = line.split_once('=')
            && let Some(block) = out.last_mut()
        {
            block.keys.push((k.trim().to_owned(), v.trim().replace("##", "#")));
        }
    }
    out
}

fn strip_comment(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '#' {
            if chars.peek() == Some(&'#') {
                chars.next();
                out.push_str("##");
                continue;
            }
            break;
        }
        out.push(c);
    }
    out
}

/// What a listener's command does, by what it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Dim,
    Lock,
    ScreensOff,
    Suspend,
}

fn action(command: &str) -> Option<Action> {
    let c = command.to_lowercase();
    if c.contains("dpms off") || c.contains("dpms, off") {
        Some(Action::ScreensOff)
    } else if c.contains("suspend") || c.contains("hibernate") {
        Some(Action::Suspend)
    } else if c.contains("lock-session") || LOCKERS.iter().any(|l| c.contains(l)) {
        Some(Action::Lock)
    } else if c.contains("brightnessctl") || c.contains("light -") || c.contains("dim") {
        Some(Action::Dim)
    } else {
        None
    }
}

/// The settings in a hypridle.conf. Listeners it doesn't recognise are left out.
pub fn parse(text: &str) -> Idle {
    let mut idle = Idle { ours: text.lines().next() == Some(HEADER), ..Idle::default() };
    let blocks = blocks(text);
    if let Some(general) = blocks.iter().find(|b| b.name == "general") {
        let lock_cmd = general.get("lock_cmd").unwrap_or("");
        idle.locker = LOCKERS.iter().find(|l| lock_cmd.contains(*l)).map(|l| (*l).to_owned()).unwrap_or_default();
        idle.lock_before_sleep = general.get("before_sleep_cmd").is_some_and(|c| action(c) == Some(Action::Lock));
    }
    for listener in blocks.iter().filter(|b| b.name == "listener") {
        let Some(timeout) = listener.get("timeout").and_then(|t| t.parse::<u32>().ok()) else { continue };
        let slot = match listener.get("on-timeout").and_then(action) {
            Some(Action::Dim) => &mut idle.dim,
            Some(Action::Lock) => &mut idle.lock,
            Some(Action::ScreensOff) => &mut idle.screens_off,
            Some(Action::Suspend) => &mut idle.suspend,
            None => continue,
        };
        // The first listener of each kind counts, as hypridle runs them all anyway.
        slot.get_or_insert(timeout);
    }
    if idle.locker.is_empty() {
        idle.locker = "hyprlock".to_owned();
    }
    idle
}

/// Lituus's hypridle.conf for these settings. `dim` says how: by the backlight (brightnessctl)
/// when the computer has one.
pub fn write(idle: &Idle, backlight: bool) -> String {
    let locker = if LOCKERS.contains(&idle.locker.as_str()) { idle.locker.as_str() } else { "hyprlock" };
    let mut out = format!("{HEADER}\n\ngeneral {{\n    lock_cmd = pidof {locker} || {locker}\n");
    if idle.lock_before_sleep {
        out.push_str("    before_sleep_cmd = loginctl lock-session\n");
    }
    out.push_str("    after_sleep_cmd = hyprctl dispatch dpms on\n}\n");
    let mut listener = |timeout: Option<u32>, on: &str, resume: Option<&str>| {
        if let Some(t) = timeout {
            out.push_str(&format!("\nlistener {{\n    timeout = {t}\n    on-timeout = {on}\n"));
            if let Some(r) = resume {
                out.push_str(&format!("    on-resume = {r}\n"));
            }
            out.push_str("}\n");
        }
    };
    if backlight {
        listener(idle.dim, "brightnessctl -s set 10%", Some("brightnessctl -r"));
    }
    listener(idle.lock, "loginctl lock-session", None);
    listener(idle.screens_off, "hyprctl dispatch dpms off", Some("hyprctl dispatch dpms on"));
    listener(idle.suspend, "systemctl suspend", None);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ML4W's hypridle.conf, as the owner has it.
    const ML4W: &str = "# Hypridle
general {
    lock_cmd = pidof hyprlock || hyprlock       # avoid starting multiple hyprlock instances.
    before_sleep_cmd = loginctl lock-session    # lock before suspend.
    after_sleep_cmd = hyprctl dispatch dpms on  # to avoid having to press a key twice to turn on the display.
}

# Screenlock
listener {
    timeout = 600                                 # in seconds
    on-timeout = loginctl lock-session            # command to run when timeout has passed
    # on-resume = notify-send \"Welcome back!\"   # command to run when activity is detected after timeout has fired.
}

# dpms
listener {
    timeout = 660                                 # in seconds
    on-timeout = hyprctl dispatch dpms off        # screen off when timeout has passed
    on-resume = hyprctl dispatch dpms on          # screen on when activity is detected after timeout has fired.
}

# Suspend
listener {
    timeout = 1800                                # in seconds
    on-timeout = systemctl suspend                # command to run when timeout has passed
}
";

    #[test]
    fn the_dotfiles_file_is_read_and_isnt_ours() {
        let idle = parse(ML4W);
        assert_eq!(idle.dim, None);
        assert_eq!(idle.lock, Some(600));
        assert_eq!(idle.screens_off, Some(660));
        assert_eq!(idle.suspend, Some(1800));
        assert!(idle.lock_before_sleep);
        assert_eq!(idle.locker, "hyprlock");
        assert!(!idle.ours);
    }

    #[test]
    fn lituus_writes_what_it_reads_back() {
        let idle = Idle {
            dim: Some(300),
            lock: Some(600),
            screens_off: Some(900),
            suspend: None,
            lock_before_sleep: true,
            locker: "hyprlock".into(),
            ours: true,
        };
        let text = write(&idle, true);
        assert!(text.starts_with(HEADER));
        assert_eq!(parse(&text), idle);
        // Without a backlight there's nothing to dim.
        assert_eq!(parse(&write(&idle, false)).dim, None);
        // Never suspending writes no listener for it.
        assert!(!text.contains("systemctl suspend"));
    }

    #[test]
    fn unknown_listeners_and_nested_blocks_are_left_out() {
        let text = "listener {\n timeout = 60\n on-timeout = notify-send hi\n}\nlistener {\n timeout = 120\n inner {\n timeout = 5\n }\n on-timeout = loginctl lock-session\n}\n";
        let idle = parse(text);
        assert_eq!(idle.lock, Some(120));
        assert_eq!((idle.dim, idle.screens_off, idle.suspend), (None, None, None));
        // A command with an escaped # keeps it.
        let text = "general {\n lock_cmd = swaylock -c ##000000\n}\n";
        assert_eq!(parse(text).locker, "swaylock");
    }
}
