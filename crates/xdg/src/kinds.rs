//! The kinds of things the Default apps page sets an app for: a web browser, images, … Each is a
//! group of file types (and link schemes); choosing its app sets them all. The terminal is a
//! kind without types: xdg-terminal-exec's list says it.

use std::collections::HashMap;

use crate::desktop::App;
use crate::mime::MimeDb;
use crate::mimeapps::{self, ListFile};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Everyday,
    Media,
}

#[derive(Clone, Copy, Debug)]
pub struct Kind {
    pub key: &'static str,
    pub label: &'static str,
    /// What it's for, when that isn't a list of extensions.
    pub hint: &'static str,
    pub icon: &'static str,
    pub group: Group,
    /// The first one is what an app must open to be offered.
    pub types: &'static [&'static str],
}

pub const KINDS: [Kind; 10] = [
    Kind {
        key: "browser",
        label: "Web browser",
        hint: "Links and web pages",
        icon: "globe",
        group: Group::Everyday,
        types: &["x-scheme-handler/http", "x-scheme-handler/https", "text/html", "application/xhtml+xml"],
    },
    Kind {
        key: "email",
        label: "Email",
        hint: "Email links (mailto:)",
        icon: "mail",
        group: Group::Everyday,
        types: &["x-scheme-handler/mailto"],
    },
    Kind {
        key: "files",
        label: "Files",
        hint: "Folders, and \u{201c}Show in folder\u{201d} in other apps",
        icon: "folder",
        group: Group::Everyday,
        types: &["inode/directory"],
    },
    Kind {
        key: "terminal",
        label: "Terminal",
        hint: "Apps that ask for one, and the Update buttons on System",
        icon: "terminal",
        group: Group::Everyday,
        types: &[],
    },
    Kind {
        key: "text",
        label: "Text editor",
        hint: "Plain text, code and config files",
        icon: "file",
        group: Group::Everyday,
        types: &[
            "text/plain",
            "text/markdown",
            "application/json",
            "application/xml",
            "application/toml",
            "application/x-yaml",
            "application/x-shellscript",
            "text/x-python",
            "text/x-csrc",
            "text/x-chdr",
            "text/x-c++src",
            "text/rust",
            "text/x-log",
        ],
    },
    Kind {
        key: "images",
        label: "Images",
        hint: "",
        icon: "picture",
        group: Group::Media,
        types: &[
            "image/png",
            "image/jpeg",
            "image/webp",
            "image/gif",
            "image/avif",
            "image/heif",
            "image/jxl",
            "image/bmp",
            "image/tiff",
            "image/svg+xml",
            "image/x-icon",
            "image/vnd.microsoft.icon",
        ],
    },
    Kind {
        key: "videos",
        label: "Videos",
        hint: "",
        icon: "film",
        group: Group::Media,
        types: &[
            "video/mp4",
            "video/x-matroska",
            "video/webm",
            "video/quicktime",
            "video/x-msvideo",
            "video/mpeg",
            "video/ogg",
            "video/x-flv",
            "video/3gpp",
            "video/x-ms-wmv",
            "video/mp2t",
            "video/x-ogm+ogg",
            "video/vnd.avi",
            "video/x-theora+ogg",
        ],
    },
    Kind {
        key: "music",
        label: "Music",
        hint: "",
        icon: "music",
        group: Group::Media,
        types: &[
            "audio/mpeg",
            "audio/flac",
            "audio/x-opus+ogg",
            "audio/ogg",
            "audio/x-vorbis+ogg",
            "audio/x-wav",
            "audio/wav",
            "audio/mp4",
            "audio/aac",
            "audio/x-m4a",
            "audio/webm",
            "audio/x-ms-wma",
            "audio/x-aiff",
        ],
    },
    Kind {
        key: "documents",
        label: "PDFs",
        hint: "PDF and other documents to read",
        icon: "doc",
        group: Group::Media,
        types: &[
            "application/pdf",
            "application/epub+zip",
            "image/vnd.djvu",
            "application/postscript",
            "application/oxps",
        ],
    },
    Kind {
        key: "archives",
        label: "Archives",
        hint: "",
        icon: "archive",
        group: Group::Media,
        types: &[
            "application/zip",
            "application/x-tar",
            "application/x-7z-compressed",
            "application/x-compressed-tar",
            "application/x-xz-compressed-tar",
            "application/x-zstd-compressed-tar",
            "application/x-bzip2-compressed-tar",
            "application/vnd.rar",
            "application/x-rar",
            "application/gzip",
            "application/x-xz",
            "application/zstd",
            "application/x-bzip2",
        ],
    },
];

pub fn kind(key: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.key == key)
}

/// The kind a file type belongs to.
pub fn kind_of(mime: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.types.contains(&mime))
}

/// A kind as it is now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KindState {
    /// Its types this computer knows (shared-mime-info has them, or an app opens them).
    pub types: Vec<String>,
    /// The app most of them open in.
    pub app: Option<String>,
    /// How many of them an app opens.
    pub opened: usize,
    /// The ones that open in another app, with it.
    pub others: Vec<(String, Option<String>)>,
    /// Where the kind's app comes from, if one file names it for all of them; `None` when no
    /// file does (the first app that opens them).
    pub from: Option<usize>,
    pub set_somewhere: bool,
}

pub fn state(kind: &Kind, files: &[ListFile], apps: &[App], db: &MimeDb) -> KindState {
    let types: Vec<String> = kind
        .types
        .iter()
        .filter(|t| {
            t.starts_with("x-scheme-handler/")
                || t.starts_with("inode/")
                || !db.extensions(t).is_empty()
                || apps.iter().any(|a| a.opens(t))
        })
        .map(|t| (*t).to_owned())
        .collect();
    let resolved: Vec<mimeapps::Resolved> = types.iter().map(|t| mimeapps::resolve(files, apps, t)).collect();
    // The most common app; on a tie, the first type's.
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for r in &resolved {
        if let Some(a) = &r.app {
            *counts.entry(a.as_str()).or_default() += 1;
        }
    }
    let best = counts.values().copied().max().unwrap_or(0);
    let app =
        resolved.iter().filter_map(|r| r.app.as_deref()).find(|a| counts.get(a) == Some(&best)).map(str::to_owned);
    let others =
        types.iter().zip(&resolved).filter(|(_, r)| r.app.is_some() && r.app != app).map(|(t, r)| (t.clone(), r.app.clone())).collect();
    let opened = resolved.iter().filter(|r| r.app.is_some()).count();
    let of_app: Vec<&mimeapps::Resolved> = resolved.iter().filter(|r| r.app == app).collect();
    let from = of_app.first().and_then(|r| r.from).filter(|f| of_app.iter().all(|r| r.from == Some(*f)));
    let set_somewhere = resolved.iter().any(|r| r.from.is_some());
    KindState { types, app, opened, others, from, set_somewhere }
}

/// The apps to offer for a kind: those that open its first type (or any of them), the kind's
/// own app first.
pub fn offered(kind: &Kind, apps: &[App]) -> Vec<String> {
    let main = kind.types.first().copied().unwrap_or_default();
    let mut out: Vec<String> = apps.iter().filter(|a| a.opens(main)).map(|a| a.id.clone()).collect();
    for a in apps {
        if kind.types.iter().any(|t| a.opens(t)) && !out.contains(&a.id) {
            out.push(a.id.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, mimes: &[&str]) -> App {
        App {
            id: id.into(),
            name: id.into(),
            mime_types: mimes.iter().map(|m| (*m).into()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_kind_with_one_type_elsewhere() {
        let files = vec![ListFile {
            own: true,
            exists: true,
            defaults: vec![
                ("image/png".into(), vec!["imv.desktop".into()]),
                ("image/jpeg".into(), vec!["imv.desktop".into()]),
                ("image/gif".into(), vec!["firefox.desktop".into()]),
            ],
            ..Default::default()
        }];
        let apps = vec![
            app("imv.desktop", &["image/png", "image/jpeg", "image/gif"]),
            app("firefox.desktop", &["image/gif", "text/html"]),
        ];
        let images = kind("images").unwrap();
        let s = state(images, &files, &apps, &MimeDb::default());
        assert_eq!(s.types, ["image/png", "image/jpeg", "image/gif"]);
        assert_eq!(s.app.as_deref(), Some("imv.desktop"));
        assert_eq!(s.others, [("image/gif".to_owned(), Some("firefox.desktop".to_owned()))]);
        assert_eq!(s.from, Some(0));
        assert_eq!(offered(images, &apps), ["imv.desktop", "firefox.desktop"]);
        assert_eq!(kind_of("image/gif").map(|k| k.key), Some("images"));
    }
}
