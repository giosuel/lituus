//! The Date & language page (D-58, S-183 and S-184): the time zone and the time, set from the
//! internet (systemd-timesyncd) or by hand, through systemd-timedated; the language and the
//! formats, for the whole system, through systemd-localed (/etc/locale.conf). Every change asks
//! for the password through polkit, so without a polkit agent running the page says which to
//! install first (S-184). It's read on a thread when the page opens; the clock every 20 seconds.

use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use slint::SharedString;
use zbus::Connection;

use crate::{DateView, HeroInfo, MainWindow, ZoneRow, model, update};

const TIMEDATE: &str = "org.freedesktop.timedate1";
const TIMEDATE_PATH: &str = "/org/freedesktop/timedate1";
const LOCALE: &str = "org.freedesktop.locale1";
const LOCALE_PATH: &str = "/org/freedesktop/locale1";

/// The formats: the categories apart from the language's messages (and its sorting).
const FORMATS: [&str; 5] = ["LC_TIME", "LC_NUMERIC", "LC_MONETARY", "LC_MEASUREMENT", "LC_PAPER"];

/// What timedated says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clock {
    pub zone: String,
    /// Set from the internet (timesyncd), whether that's possible, and whether it's in step.
    pub ntp: bool,
    pub can_ntp: bool,
    pub synced: bool,
}

/// Now, in the zone and the formats.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Now {
    /// "14:32", "Wednesday, 25 November 2026", "25.11.2026"; the zone's "CET" and "+0100"; and
    /// the fields' "2026-11-25" and "14:32".
    pub time: String,
    pub long_date: String,
    pub short_date: String,
    pub abbrev: String,
    pub offset: String,
    pub iso_date: String,
    pub iso_time: String,
}

/// The formats shown: a date, a time, a number and an amount of money.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    pub date: String,
    pub time: String,
    pub number: String,
    pub money: String,
}

/// Everything read for the page, on a thread.
#[derive(Clone, Debug, Default)]
pub struct Read {
    pub clock: Option<Clock>,
    pub locale: Vec<String>,
    pub locales: Vec<String>,
    pub languages: HashMap<String, String>,
    pub countries: HashMap<String, String>,
    pub zones: Vec<(String, String)>,
    pub now: Now,
    pub next_change: Option<String>,
    pub preview: Preview,
    pub agent: bool,
    pub install: String,
}

#[derive(Default)]
pub struct State {
    /// Read once at least; `read.clock` is None when timedated doesn't answer.
    pub loaded: bool,
    pub read: Read,
    pub working: bool,
    pub error: Option<String>,
    /// The time-zone picker is open, and its search.
    pub picking: bool,
    pub search: String,
    /// Counts the entries refused: the fields show the values in use again.
    pub refused: i32,
    /// The page was opened: read it again.
    pub reread: bool,
}

// ---- reading -------------------------------------------------------------------------------------

/// Reads the page on a thread; the window follows.
pub fn spawn(weak: slint::Weak<MainWindow>) {
    std::thread::spawn(move || {
        let read = read_all();
        let w = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |_| {
            update(&w, |app| {
                let state = &mut app.date;
                state.read = read;
                state.loaded = true;
            })
        });
    });
}

/// The clock and timedated every 20 seconds, for the hero and the sidebar ("14:32").
pub fn spawn_clock(weak: slint::Weak<MainWindow>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(20));
            let clock = block_on(async { timedated(&Connection::system().await.ok()?).await });
            let w = weak.clone();
            let posted = weak.upgrade_in_event_loop(move |_| {
                update(&w.clone(), |app| {
                    let state = &mut app.date;
                    if !state.loaded {
                        return;
                    }
                    let formats = current(&state.read.locale, "LC_TIME");
                    let zone = clock.as_ref().map(|c| c.zone.clone()).unwrap_or_default();
                    if clock.is_some() {
                        state.read.clock = clock;
                    }
                    let w = w.clone();
                    // The clock itself from `date`, off the window's thread.
                    std::thread::spawn(move || {
                        let now = now(&zone, &formats);
                        let w2 = w.clone();
                        let _ = w.upgrade_in_event_loop(move |_| update(&w2, |app| app.date.read.now = now));
                    });
                })
            });
            if posted.is_err() {
                return;
            }
        }
    });
}

fn block_on<T>(future: impl std::future::Future<Output = Option<T>>) -> Option<T> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?.block_on(future)
}

/// Everything the page shows.
fn read_all() -> Read {
    let (clock, locale) = block_on(async {
        let conn = Connection::system().await.ok()?;
        Some((timedated(&conn).await, localed(&conn).await.unwrap_or_default()))
    })
    .unwrap_or_default();
    let zone = clock.as_ref().map(|c| c.zone.clone()).unwrap_or_default();
    let formats = current(&locale, "LC_TIME");
    let iso = |name: &str| std::fs::read_to_string(format!("/usr/share/iso-codes/json/{name}")).unwrap_or_default();
    Read {
        now: now(&zone, &formats),
        next_change: next_change(&zone),
        preview: preview(&zone, &formats),
        locales: installed_locales(),
        languages: iso_names(&iso("iso_639-3.json"), "639-3"),
        countries: iso_names(&iso("iso_3166-1.json"), "3166-1"),
        zones: zones(),
        agent: agent_running(),
        install: crate::power::install_line(
            &std::fs::read_to_string("/etc/os-release").unwrap_or_default(),
            "hyprpolkitagent",
        ),
        clock,
        locale,
    }
}

async fn get_all(conn: &Connection, dest: &str, path: &str) -> Option<HashMap<String, zbus::zvariant::OwnedValue>> {
    let reply =
        conn.call_method(Some(dest), path, Some("org.freedesktop.DBus.Properties"), "GetAll", &(dest,)).await.ok()?;
    reply.body().deserialize().ok()
}

/// timedated's zone and how the time is set; None when it doesn't answer.
async fn timedated(conn: &Connection) -> Option<Clock> {
    let props = get_all(conn, TIMEDATE, TIMEDATE_PATH).await?;
    let flag = |k: &str| props.get(k).and_then(|v| bool::try_from(v.clone()).ok()).unwrap_or(false);
    Some(Clock {
        zone: props.get("Timezone").and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default(),
        ntp: flag("NTP"),
        can_ntp: flag("CanNTP"),
        synced: flag("NTPSynchronized"),
    })
}

/// localed's settings: "LANG=en_GB.UTF-8", "LC_TIME=de_CH.UTF-8"…
async fn localed(conn: &Connection) -> Option<Vec<String>> {
    let props = get_all(conn, LOCALE, LOCALE_PATH).await?;
    props.get("Locale").and_then(|v| Vec::<String>::try_from(v.clone()).ok())
}

/// The locales generated on this system, as localed takes them ("en_GB.UTF-8").
fn installed_locales() -> Vec<String> {
    let out = Command::new("localectl").arg("list-locales").output().ok();
    let mut list: Vec<String> = out
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if list.is_empty()
        && let Ok(o) = Command::new("locale").arg("-a").output()
    {
        list = String::from_utf8_lossy(&o.stdout).lines().filter_map(canonical).collect();
    }
    list.sort();
    list.dedup();
    list
}

/// `locale -a`'s "en_GB.utf8" as localed's "en_GB.UTF-8"; none for C and POSIX's other names.
fn canonical(name: &str) -> Option<String> {
    let name = name.trim();
    let (base, codeset) = name.split_once('.').unwrap_or((name, ""));
    if base.is_empty() || base == "POSIX" || (base == "C" && codeset.is_empty()) {
        return None;
    }
    Some(match codeset.to_ascii_lowercase().replace('-', "").as_str() {
        "utf8" => format!("{base}.UTF-8"),
        "" => base.to_owned(),
        _ => name.to_owned(),
    })
}

/// The names in an iso-codes file: alpha-2 and alpha-3 codes to names (the common name first).
fn iso_names(json: &str, key: &str) -> HashMap<String, String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else { return HashMap::new() };
    let mut names = HashMap::new();
    for item in value.get(key).and_then(|v| v.as_array()).into_iter().flatten() {
        let name = item.get("common_name").or_else(|| item.get("name")).and_then(|v| v.as_str()).unwrap_or_default();
        for code in ["alpha_2", "alpha_3"] {
            if let Some(c) = item.get(code).and_then(|v| v.as_str()) {
                names.entry(c.to_owned()).or_insert_with(|| name.to_owned());
            }
        }
    }
    names
}

/// The zones of tzdata's zone1970.tab, and UTC, each with its offset now ("UTC+1").
fn zones() -> Vec<(String, String)> {
    let text = std::fs::read_to_string("/usr/share/zoneinfo/zone1970.tab").unwrap_or_default();
    let mut names: Vec<String> =
        text.lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.split('\t').nth(2)).map(str::to_owned).collect();
    names.sort();
    names.push("UTC".into());
    // One shell for all of them: a `date` each.
    let script = "for z in \"$@\"; do TZ=\"$z\" date +%z; done";
    let offsets: Vec<String> = Command::new("sh")
        .args(["-c", script, "sh"])
        .args(&names)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(utc).collect())
        .unwrap_or_default();
    names.into_iter().enumerate().map(|(i, n)| (n, offsets.get(i).cloned().unwrap_or_default())).collect()
}

/// "+0100" as "UTC+1", "+0530" as "UTC+5:30", "+0000" as "UTC".
fn utc(offset: &str) -> String {
    let o = offset.trim();
    let (sign, digits) = o.split_at(o.len().min(1));
    let (h, m) = digits.split_at(digits.len().min(2));
    let (Ok(h), Ok(m)) = (h.parse::<u32>(), m.parse::<u32>()) else { return String::new() };
    match (h, m) {
        (0, 0) => "UTC".into(),
        (h, 0) => format!("UTC{sign}{h}"),
        (h, m) => format!("UTC{sign}{h}:{m:02}"),
    }
}

/// Now, from `date` in the zone and the formats.
fn now(zone: &str, formats: &str) -> Now {
    let keys = locale_keys(formats, "LC_TIME");
    let t_fmt = keys.get("t_fmt").map(|f| without_seconds(f)).unwrap_or_else(|| "%H:%M".into());
    let d_fmt = keys.get("d_fmt").cloned().unwrap_or_else(|| "%x".into());
    let format = format!("{t_fmt}\n%A, %-d %B %Y\n{d_fmt}\n%Z\n%z\n%Y-%m-%d\n%H:%M");
    let mut cmd = Command::new("date");
    cmd.arg(format!("+{format}"));
    if !zone.is_empty() {
        cmd.env("TZ", zone);
    }
    if !formats.is_empty() {
        cmd.env("LC_ALL", formats);
    }
    let text = cmd.output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let mut l = text.lines().map(str::to_owned);
    let mut next = || l.next().unwrap_or_default();
    Now {
        time: next(),
        long_date: next(),
        short_date: next(),
        abbrev: next(),
        offset: next(),
        iso_date: next(),
        iso_time: next(),
    }
}

/// A time format without its seconds: "%H:%M:%S" → "%H:%M", "%I:%M:%S %p" → "%I:%M %p", "%T" →
/// "%H:%M".
fn without_seconds(format: &str) -> String {
    format.replace("%T", "%H:%M").replace("%r", "%I:%M %p").replace(":%S", "").replace(".%S", "")
}

/// `locale -k <category>` in a locale: its keys and values, quotes taken off.
fn locale_keys(locale: &str, category: &str) -> HashMap<String, String> {
    if locale.is_empty() {
        return HashMap::new();
    }
    Command::new("locale")
        .args(["-k", category])
        .env("LC_ALL", locale)
        .output()
        .map(|o| parse_keys(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

fn parse_keys(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().trim_matches('"').to_owned()))
        .collect()
}

/// The preview in the formats: today's date with its day and the time (in the zone), 1234.56 and
/// 12.50 of money.
fn preview(zone: &str, formats: &str) -> Preview {
    let now = now(zone, formats);
    let number = number(&locale_keys(formats, "LC_NUMERIC"), 1234, 56);
    Preview {
        date: format!("{}, {}", now.long_date.split(',').next().unwrap_or_default(), now.short_date),
        time: now.time,
        number,
        money: money(&locale_keys(formats, "LC_MONETARY"), 12, 50),
    }
}

/// A number as the formats write it: "1,234.56", "1’234.56", "1 234,56". (coreutils' printf
/// leaves out the grouping for decimals.)
fn number(keys: &HashMap<String, String>, units: u64, cents: u32) -> String {
    let point = keys.get("decimal_point").filter(|p| !p.is_empty()).map(String::as_str).unwrap_or(".");
    let sep = keys.get("thousands_sep").map(String::as_str).unwrap_or_default();
    // "3", or "3;2" (India): the first group, then the others' size.
    let sizes: Vec<usize> = keys
        .get("grouping")
        .map(|g| g.split(';').filter_map(|n| n.trim().parse().ok()).filter(|n| *n > 0).collect())
        .unwrap_or_default();
    let digits = units.to_string();
    let mut groups: Vec<&str> = Vec::new();
    let mut end = digits.len();
    let mut i = 0;
    while !sep.is_empty() && !sizes.is_empty() && end > 0 {
        let size = sizes[i.min(sizes.len() - 1)];
        let start = end.saturating_sub(size);
        groups.push(&digits[start..end]);
        end = start;
        i += 1;
    }
    let whole = if groups.is_empty() {
        digits.clone()
    } else {
        groups.reverse();
        groups.join(sep)
    };
    format!("{whole}{point}{cents:02}")
}

/// An amount as the formats write money: "CHF 12.50", "12,50 €", "£12.50".
fn money(keys: &HashMap<String, String>, units: u32, cents: u32) -> String {
    let symbol = keys.get("currency_symbol").map(String::as_str).unwrap_or_default();
    let point = keys.get("mon_decimal_point").filter(|p| !p.is_empty()).map(String::as_str).unwrap_or(".");
    let amount = format!("{units}{point}{cents:02}");
    if symbol.is_empty() {
        return amount;
    }
    let before = keys.get("p_cs_precedes").is_none_or(|v| v == "1");
    let space = if keys.get("p_sep_by_space").is_some_and(|v| v == "1") { " " } else { "" };
    if before { format!("{symbol}{space}{amount}") } else { format!("{amount}{space}{symbol}") }
}

/// The next change between summer and standard time: "summer time from 29 March".
fn next_change(zone: &str) -> Option<String> {
    if zone.is_empty() {
        return None;
    }
    let year: i32 = Command::new("date")
        .arg("+%Y")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())?;
    let out = Command::new("zdump").args(["-v", "-c", &format!("{year},{}", year + 2), zone]).output().ok()?;
    let now: i64 = Command::new("date")
        .arg("+%s")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())?;
    let (summer, day, month) = parse_change(&String::from_utf8_lossy(&out.stdout), now)?;
    Some(format!("{} time from {day} {month}", if summer { "summer" } else { "standard" }))
}

/// From `zdump -v`, the first change after `now` (seconds since 1970): to summer time or not,
/// and the local day and month it happens ("29", "March").
fn parse_change(zdump: &str, now: i64) -> Option<(bool, u32, &'static str)> {
    let mut before: Option<bool> = None;
    for line in zdump.lines() {
        // "Europe/Zurich  Sun Mar 29 00:59:59 2026 UT = Sun Mar 29 01:59:59 2026 CET isdst=0 gmtoff=3600"
        let Some((ut, local)) = line.split_once(" UT = ") else { continue };
        let words: Vec<&str> = ut.split_whitespace().collect();
        let [.., month, day, time, year] = words[..] else { continue };
        let Some(at) = epoch(year, month, day, time) else { continue };
        let Some(dst) = local.split_whitespace().find_map(|w| w.strip_prefix("isdst=")).map(|v| v == "1") else {
            continue;
        };
        if at <= now {
            before = Some(dst);
            continue;
        }
        match before {
            Some(b) if b != dst => {
                let l: Vec<&str> = local.split_whitespace().collect();
                let (m, d) = (l.get(1)?, l.get(2)?.parse().ok()?);
                return Some((dst, d, month_name(m)?));
            }
            _ => before = Some(dst),
        }
    }
    None
}

fn month_index(m: &str) -> Option<u32> {
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
        .iter()
        .position(|n| *n == m)
        .map(|i| i as u32 + 1)
}

fn month_name(m: &str) -> Option<&'static str> {
    const NAMES: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    month_index(m).map(|i| NAMES[i as usize - 1])
}

/// Seconds since 1970 for a UTC date and "HH:MM:SS".
fn epoch(year: &str, month: &str, day: &str, time: &str) -> Option<i64> {
    let (y, m, d): (i64, u32, i64) = (year.parse().ok()?, month_index(month)?, day.parse().ok()?);
    let mut hms = time.split(':').map(|p| p.parse::<i64>().ok());
    let (h, mi, s) = (hms.next()??, hms.next()??, hms.next()??);
    Some(days(y, m, d) * 86_400 + h * 3600 + mi * 60 + s)
}

/// Days since 1970-01-01 (Howard Hinnant's days_from_civil).
fn days(y: i64, m: u32, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// A polkit agent runs for this user: hyprpolkitagent, polkit-gnome, KDE's, lxqt-policykit, soteria…
fn agent_running() -> bool {
    let me = std::fs::metadata("/proc/self").map(|m| std::os::unix::fs::MetadataExt::uid(&m)).ok();
    let Ok(dir) = std::fs::read_dir("/proc") else { return false };
    dir.flatten().any(|e| {
        e.file_name().to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
            && std::fs::metadata(e.path()).map(|m| std::os::unix::fs::MetadataExt::uid(&m)).ok() == me
            && std::fs::read_to_string(e.path().join("cmdline"))
                .ok()
                .and_then(|c| c.split('\0').next().map(str::to_owned))
                .is_some_and(|c| is_agent(&c))
    })
}

/// A program's path or name that is a polkit agent.
fn is_agent(program: &str) -> bool {
    let name = program.rsplit('/').next().unwrap_or(program).to_ascii_lowercase();
    (name.contains("polkit") || name.contains("policykit") || name == "soteria") && name != "polkitd"
}

// ---- locales -------------------------------------------------------------------------------------

/// A category's locale in localed's settings: the category's own, else LANG's.
fn current(locale: &[String], key: &str) -> String {
    let get =
        |k: &str| locale.iter().find_map(|l| l.strip_prefix(k).and_then(|v| v.strip_prefix('='))).map(str::to_owned);
    get(key).or_else(|| get("LANG")).unwrap_or_default()
}

/// localed's settings with the language or the formats changed. The formats are left out when
/// they're the language's own; other categories (LC_MESSAGES…) stay.
fn with(locale: &[String], language: Option<&str>, formats: Option<&str>) -> Vec<String> {
    let lang = language.map(str::to_owned).unwrap_or_else(|| current(locale, "LANG"));
    let formats = formats.map(str::to_owned).unwrap_or_else(|| current(locale, "LC_TIME"));
    let mut out = vec![format!("LANG={lang}")];
    out.extend(
        locale
            .iter()
            .filter(|l| !l.starts_with("LANG=") && !FORMATS.iter().any(|f| l.starts_with(&format!("{f}="))))
            .cloned(),
    );
    if formats != lang && !formats.is_empty() {
        out.extend(FORMATS.iter().map(|f| format!("{f}={formats}")));
    }
    out
}

/// "en_GB.UTF-8" → ("en", "GB").
fn split(locale: &str) -> (&str, &str) {
    let base = locale.split(['.', '@']).next().unwrap_or(locale);
    base.split_once('_').unwrap_or((base, ""))
}

/// "English (United Kingdom)" for a language.
fn language_name(locale: &str, read: &Read) -> String {
    let (lang, country) = split(locale);
    match (read.languages.get(lang), read.countries.get(country)) {
        (Some(l), Some(c)) => format!("{l} ({c})"),
        (Some(l), None) => l.clone(),
        _ => locale.to_owned(),
    }
}

/// "Switzerland (German)" for formats.
fn formats_name(locale: &str, read: &Read) -> String {
    let (lang, country) = split(locale);
    match (read.countries.get(country), read.languages.get(lang)) {
        (Some(c), Some(l)) => format!("{c} ({l})"),
        (None, Some(l)) => l.clone(),
        _ => locale.to_owned(),
    }
}

// ---- changing ------------------------------------------------------------------------------------

/// Why a call failed, in words.
fn problem(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(name, _, _)
            if name.as_str().ends_with("NotAuthorized") || name.as_str().contains("Interactive") =>
        {
            "The password wasn't given, so nothing changed.".into()
        }
        zbus::Error::MethodError(_, Some(text), _) => text,
        other => other.to_string(),
    }
}

/// A change to timedated or localed, on a thread; then the page is read again, or says why not.
fn change(weak: &slint::Weak<MainWindow>, state: &mut State, call: Call) {
    state.working = true;
    state.error = None;
    let weak = weak.clone();
    std::thread::spawn(move || {
        let done = block_on(async {
            let conn = match Connection::system().await {
                Ok(c) => c,
                Err(e) => return Some(Err(e.to_string())),
            };
            // `true`: polkit may ask for the password (interactive).
            let reply = match &call {
                Call::Zone(z) => {
                    conn.call_method(Some(TIMEDATE), TIMEDATE_PATH, Some(TIMEDATE), "SetTimezone", &(z.as_str(), true))
                        .await
                }
                Call::Ntp(on) => {
                    conn.call_method(Some(TIMEDATE), TIMEDATE_PATH, Some(TIMEDATE), "SetNTP", &(*on, true)).await
                }
                Call::Time(usec) => {
                    conn.call_method(Some(TIMEDATE), TIMEDATE_PATH, Some(TIMEDATE), "SetTime", &(*usec, false, true))
                        .await
                }
                Call::Locale(l) => {
                    conn.call_method(Some(LOCALE), LOCALE_PATH, Some(LOCALE), "SetLocale", &(l, true)).await
                }
            };
            Some(reply.map(|_| ()).map_err(problem))
        })
        .unwrap_or_else(|| Err("Couldn't reach the system bus.".into()));
        let read = read_all();
        let w = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |_| {
            update(&w, |app| {
                let state = &mut app.date;
                state.working = false;
                state.read = read;
                if let Err(e) = done {
                    state.error = Some(e);
                    state.refused += 1;
                }
            })
        });
    });
}

enum Call {
    Zone(String),
    Ntp(bool),
    /// Microseconds since 1970, UTC.
    Time(i64),
    Locale(Vec<String>),
}

/// "2026-11-25" or "25.11.2026" as (year, month, day).
fn parse_date(text: &str) -> Option<(i32, u32, u32)> {
    let t = text.trim();
    let (y, m, d) = if let Some((y, rest)) = t.split_once('-') {
        let (m, d) = rest.split_once('-')?;
        (y, m, d)
    } else {
        let mut p = t.split('.');
        let (d, m, y) = (p.next()?, p.next()?, p.next()?);
        (y, m, d)
    };
    let (y, m, d) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    ((1970..=2100).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// "14:32" as (hours, minutes).
fn parse_clock(text: &str) -> Option<(u32, u32)> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

/// A local date and time in the zone as microseconds since 1970 (`date -d`, which knows the zone's
/// rules).
fn local_usec(zone: &str, (y, mo, d): (i32, u32, u32), (h, mi): (u32, u32)) -> Option<i64> {
    let out = Command::new("date")
        .env("TZ", zone)
        .args(["-d", &format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}"), "+%s"])
        .output()
        .ok()?;
    let secs: i64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some(secs * 1_000_000)
}

/// What a click or an entry on the page does: (verb, number, text).
pub fn act(weak: &slint::Weak<MainWindow>, state: &mut State, verb: &str, value: i32, text: &str) {
    let zone = state.read.clock.as_ref().map(|c| c.zone.clone()).unwrap_or_default();
    match verb {
        "dismiss" | "copied" => state.error = None,
        "automatic" => {
            let on = !state.read.clock.as_ref().is_some_and(|c| c.ntp);
            change(weak, state, Call::Ntp(on));
        }
        "zone-open" => {
            state.picking = true;
            state.search.clear();
        }
        "zone-search" => state.search = text.to_owned(),
        "zone-close" => state.picking = false,
        "zone" => {
            state.picking = false;
            if text != zone {
                change(weak, state, Call::Zone(text.to_owned()));
            }
        }
        "date" | "time" => {
            let date = if verb == "date" { text } else { &state.read.now.iso_date };
            let time = if verb == "time" { text } else { &state.read.now.iso_time };
            match (parse_date(date), parse_clock(time)) {
                (Some(d), Some(t)) => match local_usec(&zone, d, t) {
                    Some(usec) => change(weak, state, Call::Time(usec)),
                    None => state.error = Some("That date and time don't exist in this time zone.".into()),
                },
                (None, _) => {
                    state.error = Some("Type a date as 2026-11-25.".into());
                    state.refused += 1;
                }
                (_, None) => {
                    state.error = Some("Type a time as 14:32.".into());
                    state.refused += 1;
                }
            }
        }
        "language" | "formats" => {
            let Some(chosen) = state.read.locales.get(value as usize).cloned() else { return };
            let locale = if verb == "language" {
                with(&state.read.locale, Some(&chosen), None)
            } else {
                with(&state.read.locale, None, Some(&chosen))
            };
            if locale != state.read.locale {
                change(weak, state, Call::Locale(locale));
            }
        }
        _ => {}
    }
}

// ---- the page ------------------------------------------------------------------------------------

fn s(text: impl AsRef<str>) -> SharedString {
    SharedString::from(text.as_ref())
}

/// The sidebar's status: the time.
pub fn status(state: &State) -> String {
    state.read.now.time.clone()
}

/// "Set automatically · English (United Kingdom), Swiss formats".
fn sub(state: &State) -> String {
    let read = &state.read;
    let how = match &read.clock {
        Some(c) if c.ntp => "Set automatically",
        Some(_) => "Set by hand",
        None => return String::new(),
    };
    let lang = current(&read.locale, "LANG");
    let formats = current(&read.locale, "LC_TIME");
    if lang.is_empty() {
        return how.to_owned();
    }
    let mut line = format!("{how} · {}", language_name(&lang, read));
    if formats != lang {
        let (_, country) = split(&formats);
        let place = read.countries.get(country).cloned().unwrap_or_else(|| formats.clone());
        line.push_str(&format!(", {place} formats"));
    }
    line
}

pub fn view(state: &State) -> DateView {
    let mut v = DateView { error: s(state.error.clone().unwrap_or_default()), ..DateView::default() };
    let read = &state.read;
    if !state.loaded {
        v.state = s("loading");
        return v;
    }
    let Some(clock) = &read.clock else {
        v.state = s("missing");
        v.hero = HeroInfo {
            icon: s("clock"),
            name: s(&read.now.time),
            meta: s(&read.now.long_date),
            tone: s("muted"),
            detail: s("systemd-timedated isn't running"),
        };
        return v;
    };
    v.state = s("list");
    v.sub = s(sub(state));
    let zone_line = format!("{}, {}", read.now.abbrev, utc(&read.now.offset));
    v.hero = HeroInfo {
        icon: s("clock"),
        name: s(&read.now.time),
        meta: s(format!("{} · {}", read.now.long_date, clock.zone)),
        tone: s("accent"),
        detail: s(format!(
            "{zone_line} · {}",
            match (clock.ntp, clock.synced) {
                (true, true) => "synced",
                (true, false) => "not synced yet",
                (false, _) => "set by hand",
            }
        )),
    };
    v.working = state.working;
    v.agent = read.agent;
    v.agent_command = s(&read.install);
    v.automatic = clock.ntp;
    v.can_ntp = clock.can_ntp;
    v.zone = s(&clock.zone);
    v.zone_text = s(match &read.next_change {
        Some(next) => format!("{zone_line}; {next}"),
        None => zone_line,
    });
    v.date = s(&read.now.short_date);
    v.time = s(&read.now.time);
    v.date_input = s(&read.now.iso_date);
    v.time_input = s(&read.now.iso_time);
    let lang = current(&read.locale, "LANG");
    let formats = current(&read.locale, "LC_TIME");
    v.languages = model(read.locales.iter().map(|l| s(language_name(l, read))).collect());
    v.formats = model(read.locales.iter().map(|l| s(formats_name(l, read))).collect());
    v.language = read.locales.iter().position(|l| *l == lang).map_or(-1, |i| i as i32);
    v.format = read.locales.iter().position(|l| *l == formats).map_or(-1, |i| i as i32);
    v.preview_date = s(&read.preview.date);
    v.preview_time = s(&read.preview.time);
    v.preview_number = s(&read.preview.number);
    v.preview_money = s(&read.preview.money);
    v.refused = state.refused;
    v.picking = state.picking;
    v.search = s(&state.search);
    let search = state.search.to_lowercase().replace(' ', "_");
    v.zones = model(
        read.zones
            .iter()
            .filter(|(z, o)| {
                search.is_empty() || z.to_lowercase().contains(&search) || o.to_lowercase().contains(&search)
            })
            .map(|(z, o)| ZoneRow { name: s(z), detail: s(o), selected: *z == clock.zone })
            .collect(),
    );
    v
}

pub fn render(window: &MainWindow, state: &State) {
    window.set_date(view(state));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_names_and_settings() {
        assert_eq!(canonical("en_GB.utf8").as_deref(), Some("en_GB.UTF-8"));
        assert_eq!(canonical("C.utf8").as_deref(), Some("C.UTF-8"));
        assert_eq!(canonical("POSIX"), None);
        assert_eq!(canonical("C"), None);
        assert_eq!(split("de_CH.UTF-8"), ("de", "CH"));
        assert_eq!(split("ca_ES@valencia"), ("ca", "ES"));
        let read = Read {
            languages: [("en".into(), "English".into()), ("de".into(), "German".into())].into(),
            countries: [("GB".into(), "United Kingdom".into()), ("CH".into(), "Switzerland".into())].into(),
            ..Read::default()
        };
        assert_eq!(language_name("en_GB.UTF-8", &read), "English (United Kingdom)");
        assert_eq!(formats_name("de_CH.UTF-8", &read), "Switzerland (German)");
        assert_eq!(language_name("C.UTF-8", &read), "C.UTF-8");
        // The formats go with the language until they're another, and other categories stay.
        let locale = vec!["LANG=en_GB.UTF-8".to_owned(), "LC_MESSAGES=en_GB.UTF-8".to_owned()];
        let swiss = with(&locale, None, Some("de_CH.UTF-8"));
        assert_eq!(swiss[0], "LANG=en_GB.UTF-8");
        assert!(swiss.contains(&"LC_MESSAGES=en_GB.UTF-8".to_owned()));
        assert!(
            swiss.contains(&"LC_TIME=de_CH.UTF-8".to_owned()) && swiss.contains(&"LC_MONETARY=de_CH.UTF-8".to_owned())
        );
        assert_eq!(current(&swiss, "LC_TIME"), "de_CH.UTF-8");
        let back = with(&swiss, None, Some("en_GB.UTF-8"));
        assert_eq!(back, locale);
        assert_eq!(with(&swiss, Some("de_CH.UTF-8"), None)[0], "LANG=de_CH.UTF-8");
        let names = iso_names(
            r#"{"3166-1": [{"alpha_2": "GB", "alpha_3": "GBR", "name": "United Kingdom of Great Britain and Northern Ireland", "common_name": "United Kingdom"}]}"#,
            "3166-1",
        );
        assert_eq!(names.get("GB").map(String::as_str), Some("United Kingdom"));
    }

    #[test]
    fn formats_preview_parts() {
        let keys = parse_keys("currency_symbol=\"CHF\"\nmon_decimal_point=\".\"\np_cs_precedes=1\np_sep_by_space=1\n");
        assert_eq!(money(&keys, 12, 50), "CHF 12.50");
        let euro = parse_keys("currency_symbol=\"€\"\nmon_decimal_point=\",\"\np_cs_precedes=0\np_sep_by_space=1\n");
        assert_eq!(money(&euro, 12, 50), "12,50 €");
        let pound = parse_keys("currency_symbol=\"£\"\nmon_decimal_point=\".\"\np_cs_precedes=1\np_sep_by_space=0\n");
        assert_eq!(money(&pound, 12, 50), "£12.50");
        assert_eq!(without_seconds("%H:%M:%S"), "%H:%M");
        assert_eq!(without_seconds("%T"), "%H:%M");
        assert_eq!(without_seconds("%I:%M:%S %p"), "%I:%M %p");
        assert_eq!(without_seconds("%r"), "%I:%M %p");
        let us = parse_keys("decimal_point=\".\"\nthousands_sep=\",\"\ngrouping=3\n");
        assert_eq!(number(&us, 1234, 56), "1,234.56");
        let swiss = parse_keys("decimal_point=\".\"\nthousands_sep=\"’\"\ngrouping=3;3\n");
        assert_eq!(number(&swiss, 1234567, 5), "1’234’567.05");
        let india = parse_keys("decimal_point=\".\"\nthousands_sep=\",\"\ngrouping=3;2\n");
        assert_eq!(number(&india, 1234567, 0), "12,34,567.00");
        assert_eq!(number(&parse_keys(""), 1234, 56), "1234.56");
        assert_eq!(utc("+0100"), "UTC+1");
        assert_eq!(utc("+0530"), "UTC+5:30");
        assert_eq!(utc("-0300"), "UTC-3");
        assert_eq!(utc("+0000"), "UTC");
    }

    #[test]
    fn the_next_change_to_summer_time() {
        let zdump = "Europe/Zurich  Sun Oct 26 00:59:59 2025 UT = Sun Oct 26 02:59:59 2025 CEST isdst=1 gmtoff=7200\n\
                     Europe/Zurich  Sun Oct 26 01:00:00 2025 UT = Sun Oct 26 02:00:00 2025 CET isdst=0 gmtoff=3600\n\
                     Europe/Zurich  Sun Mar 29 00:59:59 2026 UT = Sun Mar 29 01:59:59 2026 CET isdst=0 gmtoff=3600\n\
                     Europe/Zurich  Sun Mar 29 01:00:00 2026 UT = Sun Mar 29 03:00:00 2026 CEST isdst=1 gmtoff=7200\n\
                     Europe/Zurich  Sun Oct 25 00:59:59 2026 UT = Sun Oct 25 02:59:59 2026 CEST isdst=1 gmtoff=7200\n\
                     Europe/Zurich  Sun Oct 25 01:00:00 2026 UT = Sun Oct 25 02:00:00 2026 CET isdst=0 gmtoff=3600\n";
        // 25 November 2025: summer time next, from 29 March.
        let nov = days(2025, 11, 25) * 86_400;
        assert_eq!(parse_change(zdump, nov), Some((true, 29, "March")));
        // 1 May 2026: standard time from 25 October.
        assert_eq!(parse_change(zdump, days(2026, 5, 1) * 86_400), Some((false, 25, "October")));
        assert_eq!(days(1970, 1, 1), 0);
        assert_eq!(epoch("2026", "Mar", "29", "01:00:00"), Some(1_774_746_000));
        // UTC has no changes.
        assert_eq!(parse_change("", nov), None);
    }

    #[test]
    fn typed_dates_times_and_agents() {
        assert_eq!(parse_date("2026-11-25"), Some((2026, 11, 25)));
        assert_eq!(parse_date("25.11.2026"), Some((2026, 11, 25)));
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("tomorrow"), None);
        assert_eq!(parse_clock("14:32"), Some((14, 32)));
        assert_eq!(parse_clock("24:00"), None);
        assert!(is_agent("/usr/lib/hyprpolkitagent/hyprpolkitagent"));
        assert!(is_agent("/usr/lib/polkit-gnome/polkit-gnome-authentication-agent-1"));
        assert!(is_agent("lxqt-policykit-agent"));
        assert!(!is_agent("/usr/lib/polkit-1/polkitd"));
        assert!(!is_agent("firefox"));
    }
}
