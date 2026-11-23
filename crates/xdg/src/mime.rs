//! shared-mime-info's file types: their extensions (mime/globs2) and names (mime/<type>.xml's
//! first comment without a language), in the data folders, the user's first.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::Dirs;

#[derive(Clone, Debug, Default)]
pub struct MimeDb {
    /// Extensions by type, "png" for "*.png", most likely first.
    pub extensions: BTreeMap<String, Vec<String>>,
    mime_dirs: Vec<PathBuf>,
}

impl MimeDb {
    pub fn load(dirs: &Dirs) -> MimeDb {
        let mime_dirs: Vec<PathBuf> = dirs.data().map(|d| d.join("mime")).collect();
        let mut weighted: BTreeMap<String, Vec<(u32, String)>> = BTreeMap::new();
        for dir in mime_dirs.iter().rev() {
            let text = std::fs::read_to_string(dir.join("globs2")).unwrap_or_default();
            for line in text.lines().filter(|l| !l.starts_with('#')) {
                let mut parts = line.splitn(3, ':');
                let (Some(w), Some(mime), Some(glob)) = (parts.next(), parts.next(), parts.next()) else { continue };
                // Only "*.ext"; the flags after a further ':' aren't part of it.
                let glob = glob.split(':').next().unwrap_or(glob);
                let Some(ext) = glob.strip_prefix("*.").filter(|e| !e.contains(['*', '?', '['])) else { continue };
                let list = weighted.entry(mime.into()).or_default();
                if !list.iter().any(|(_, e)| e == ext) {
                    list.push((w.parse().unwrap_or(50), ext.to_owned()));
                }
            }
        }
        let extensions = weighted
            .into_iter()
            .map(|(m, mut l)| {
                l.sort_by_key(|e| std::cmp::Reverse(e.0));
                (m, l.into_iter().map(|(_, e)| e).collect())
            })
            .collect();
        MimeDb { extensions, mime_dirs }
    }

    pub fn extensions(&self, mime: &str) -> &[String] {
        self.extensions.get(mime).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Its name, "PNG image"; else the type itself.
    pub fn name(&self, mime: &str) -> String {
        self.mime_dirs
            .iter()
            .find_map(|d| {
                let text = std::fs::read_to_string(d.join(format!("{mime}.xml"))).ok()?;
                text.split("<comment>").nth(1)?.split("</comment>").next().map(unescape)
            })
            .unwrap_or_else(|| mime.to_owned())
    }

    /// Whether a type matches what someone typed: part of its name or type, or an extension
    /// (with or without the dot).
    pub fn matches(&self, mime: &str, name: &str, query: &str) -> bool {
        let q = query.trim().trim_start_matches('.').to_lowercase();
        if q.is_empty() {
            return false;
        }
        mime.contains(&q)
            || name.to_lowercase().contains(&q)
            || self.extensions(mime).iter().any(|e| e.to_lowercase() == q)
    }
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir;

    #[test]
    fn names_and_extensions() {
        let root = test_dir::with(
            "mime",
            &[
                (
                    "usr/share/mime/globs2",
                    "# x\n50:image/png:*.png\n80:text/markdown:*.md\n50:text/markdown:*.markdown\n50:text/x-readme:README*\n",
                ),
                (
                    "usr/share/mime/image/png.xml",
                    "<mime-type type=\"image/png\"><comment>PNG image</comment><comment xml:lang=\"de\">PNG-Bild</comment></mime-type>",
                ),
            ],
        );
        let db = MimeDb::load(&test_dir::dirs(&root));
        assert_eq!(db.extensions("text/markdown"), ["md", "markdown"]);
        assert!(db.extensions("text/x-readme").is_empty());
        assert_eq!(db.name("image/png"), "PNG image");
        assert_eq!(db.name("image/x-none"), "image/x-none");
        assert!(db.matches("image/png", "PNG image", ".png"));
        assert!(db.matches("image/png", "PNG image", "image/p"));
        assert!(db.matches("text/markdown", "Markdown document", "markdown"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
