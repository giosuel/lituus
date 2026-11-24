//! The pictures in a folder, its subfolders with a few of theirs, and small copies of them for a
//! grid, kept in a cache folder so a folder of 6K pictures opens quickly the second time.

use std::fs::{OpenOptions, Permissions};
use std::hash::{Hash, Hasher};
use std::io::{BufWriter, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// What both programs show (awww has more behind build options; hyprpaper checks these).
const EXTENSIONS: [&str; 7] = ["jpg", "jpeg", "png", "webp", "gif", "bmp", "jfif"];

/// A picture file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub path: PathBuf,
    pub name: String,
    pub modified: SystemTime,
}

impl Picture {
    /// An animated GIF moves only with awww.
    pub fn gif(&self) -> bool {
        self.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gif"))
    }
}

/// A folder in the folder: how many pictures it has, and its first four for its tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subfolder {
    pub path: PathBuf,
    pub name: String,
    pub count: usize,
    pub first: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folder {
    pub path: PathBuf,
    pub subfolders: Vec<Subfolder>,
    pub pictures: Vec<Picture>,
    /// Files that aren't pictures (videos, documents), for a folder that has nothing else.
    pub others: usize,
}

pub fn is_picture(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

fn hidden(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'))
}

fn name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The pictures directly in `dir` (or, with `deep`, in its subfolders too, up to four down),
/// sorted by name.
fn pictures_in(dir: &Path, deep: bool, depth: usize, out: &mut Vec<Picture>, others: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| !hidden(p)).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            if deep && depth < 4 {
                pictures_in(&p, deep, depth + 1, out, others);
            }
        } else if is_picture(&p) {
            let modified = p.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            out.push(Picture { name: name(&p), path: p, modified });
        } else {
            *others += 1;
        }
    }
}

/// What's in a folder: its subfolders that have pictures (unless `deep` takes their pictures in
/// with the folder's own), and its pictures.
pub fn read_folder(dir: &Path, deep: bool) -> std::io::Result<Folder> {
    std::fs::read_dir(dir)?;
    let mut pictures = Vec::new();
    let mut others = 0;
    pictures_in(dir, deep, 0, &mut pictures, &mut others);
    let mut subfolders = Vec::new();
    if !deep {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !hidden(p))
            .collect();
        dirs.sort();
        for d in dirs {
            let mut inside = Vec::new();
            let mut skipped = 0;
            pictures_in(&d, true, 1, &mut inside, &mut skipped);
            if !inside.is_empty() {
                subfolders.push(Subfolder {
                    name: name(&d),
                    count: inside.len(),
                    first: inside.iter().take(4).map(|p| p.path.clone()).collect(),
                    path: d,
                });
            }
        }
    }
    Ok(Folder { path: dir.to_path_buf(), subfolders, pictures, others })
}

/// A picture's size in pixels, read from its header.
pub fn dimensions(path: &Path) -> Option<(u32, u32)> {
    image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_dimensions().ok()
}

/// A picture, decoded by what's in it rather than its name: wallpapers are often saved with
/// another format's extension (a JPEG called .png), which `image::open` refuses.
fn open(path: &Path) -> Result<image::DynamicImage, String> {
    image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())
}

/// A thumbnail not used for this long goes (`prune_thumbnails`).
const UNUSED: Duration = Duration::from_secs(30 * 24 * 3600);
/// How often a thumbnail in use is marked so.
const DAY: Duration = Duration::from_secs(24 * 3600);

/// A small copy of a picture, `width` × `height`, cropped to fill it, in `cache`; made once for
/// each version of the file. A JPEG, a tenth of a photo's PNG, or a PNG if it has see-through
/// parts. Only the user can read the cache, as the freedesktop thumbnail spec asks: a thumbnail
/// shows what a private picture holds.
pub fn thumbnail(path: &Path, cache: &Path, width: u32, height: u32) -> Result<PathBuf, String> {
    let meta = path.metadata().map_err(|e| e.to_string())?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (path, meta.len(), meta.modified().ok(), width, height).hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());
    if let Some(done) = ["jpg", "png"].iter().map(|x| cache.join(format!("{key}.{x}"))).find(|t| t.exists()) {
        used(&done);
        return Ok(done);
    }
    let img = open(path)?;
    // Lanczos: a sharper small copy than Triangle's, made once per picture.
    let small = img.resize_to_fill(width, height, image::imageops::FilterType::Lanczos3);
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(cache).map_err(|e| e.to_string())?;
    // One made before Lituus kept it private.
    let _ = std::fs::set_permissions(cache, Permissions::from_mode(0o700));
    let clear = small.color().has_alpha() && small.to_rgba8().pixels().any(|p| p[3] < u8::MAX);
    let out = cache.join(format!("{key}.{}", if clear { "png" } else { "jpg" }));
    // Through a temporary file, so a half-written one is never taken for done; one of this
    // call's own, so two threads making the same thumbnail don't write into one file and rename
    // it from under each other.
    static MADE: AtomicU64 = AtomicU64::new(0);
    let tmp = cache.join(format!("{key}.{}.{}.tmp", std::process::id(), MADE.fetch_add(1, Ordering::Relaxed)));
    let saved = (|| -> Result<(), String> {
        let file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
        let mut w = BufWriter::new(file);
        if clear {
            small.write_with_encoder(image::codecs::png::PngEncoder::new(&mut w))
        } else {
            small.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, 85))
        }
        .map_err(|e| e.to_string())?;
        w.flush().map_err(|e| e.to_string())?;
        drop(w);
        std::fs::rename(&tmp, &out).map_err(|e| e.to_string())
    })();
    if saved.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    saved.map(|()| out)
}

/// Marks a thumbnail as used now, for `prune_thumbnails`: once a day, not a write for each
/// picture each time the page opens.
fn used(thumb: &Path) {
    let stale = thumb.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|age| age > DAY));
    if stale && let Ok(file) = OpenOptions::new().write(true).open(thumb) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// Clears out the cache of thumbnails: those not used for a month, what an interrupted save left
/// behind, and, beyond `limit` bytes, those used longest ago.
pub fn prune_thumbnails(cache: &Path, limit: u64) {
    let Ok(entries) = std::fs::read_dir(cache) else { return };
    let mut kept: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let path = entry.path();
        let used = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let age = used.elapsed().unwrap_or_default();
        // Another thread's may be in the making still.
        let tmp = path.extension().is_some_and(|x| x == "tmp");
        if age > UNUSED || (tmp && age > DAY) {
            let _ = std::fs::remove_file(&path);
        } else if !tmp {
            kept.push((used, meta.len(), path));
        }
    }
    kept.sort_by_key(|(used, _, _)| std::cmp::Reverse(*used));
    let mut total = 0;
    for (_, len, path) in kept {
        total += len;
        if total > limit {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_with_subfolders_and_thumbnails() {
        let d = std::env::temp_dir().join(format!("lituus-wallpaper-pictures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for sub in ["Nature", "Space/deep", "Empty", ".hidden"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        let png =
            |p: &Path, w: u32, h: u32| image::RgbImage::from_pixel(w, h, image::Rgb([10, 120, 200])).save(p).unwrap();
        png(&d.join("b.png"), 64, 36);
        png(&d.join("a.jpg"), 32, 18);
        png(&d.join("Nature/fjord.png"), 8, 8);
        png(&d.join("Space/deep/nebula.png"), 8, 8);
        png(&d.join(".hidden/x.png"), 8, 8);
        // A JPEG saved as .png: still read, by what's in it.
        image::RgbImage::from_pixel(40, 30, image::Rgb([200, 80, 10]))
            .save_with_format(d.join("c.png"), image::ImageFormat::Jpeg)
            .unwrap();
        assert_eq!(dimensions(&d.join("c.png")), Some((40, 30)));
        assert!(thumbnail(&d.join("c.png"), &d.join(".cache"), 16, 9).is_ok());
        std::fs::remove_file(d.join("c.png")).unwrap();
        std::fs::write(d.join("clip.mp4"), "").unwrap();
        let f = read_folder(&d, false).unwrap();
        assert_eq!(f.pictures.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["a.jpg", "b.png"]);
        assert_eq!(
            f.subfolders.iter().map(|s| (s.name.as_str(), s.count)).collect::<Vec<_>>(),
            [("Nature", 1), ("Space", 1)]
        );
        assert_eq!(f.others, 1);
        let deep = read_folder(&d, true).unwrap();
        assert_eq!(deep.pictures.len(), 4);
        assert!(deep.subfolders.is_empty());
        assert_eq!(dimensions(&d.join("b.png")), Some((64, 36)));
        let cache = d.join("cache");
        let t = thumbnail(&d.join("b.png"), &cache, 32, 18).unwrap();
        assert_eq!(dimensions(&t), Some((32, 18)));
        // A JPEG, the user's alone.
        assert_eq!(t.extension().unwrap(), "jpg");
        assert_eq!(std::fs::metadata(&cache).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&t).unwrap().permissions().mode() & 0o777, 0o600);
        // Made once.
        assert_eq!(thumbnail(&d.join("b.png"), &cache, 32, 18).unwrap(), t);
        assert!(thumbnail(&d.join("clip.mp4"), &cache, 32, 18).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_see_through_picture_keeps_its_png() {
        let d = std::env::temp_dir().join(format!("lituus-wallpaper-clear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        image::RgbaImage::from_pixel(32, 18, image::Rgba([10, 120, 200, 0])).save(d.join("clear.png")).unwrap();
        // Saved with an alpha channel, but opaque: a JPEG.
        image::RgbaImage::from_pixel(32, 18, image::Rgba([10, 120, 200, 255])).save(d.join("opaque.png")).unwrap();
        assert_eq!(thumbnail(&d.join("clear.png"), &d.join("cache"), 16, 9).unwrap().extension().unwrap(), "png");
        assert_eq!(thumbnail(&d.join("opaque.png"), &d.join("cache"), 16, 9).unwrap().extension().unwrap(), "jpg");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_cache_is_cleared_out() {
        let d = std::env::temp_dir().join(format!("lituus-wallpaper-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let now = SystemTime::now();
        let file = |name: &str, bytes: usize, ago: u64| {
            std::fs::write(d.join(name), vec![0; bytes]).unwrap();
            let f = OpenOptions::new().write(true).open(d.join(name)).unwrap();
            f.set_modified(now - Duration::from_secs(ago)).unwrap();
        };
        let hours = 3600;
        file("unused.jpg", 10, 40 * 24 * hours);
        file("left.1.2.tmp", 10, 2 * 24 * hours);
        file("making.1.3.tmp", 10, 10);
        file("old.jpg", 10, 3 * hours);
        file("older.png", 10, 4 * hours);
        file("new.jpg", 10, 1);
        file("newer.jpg", 10, 0);
        prune_thumbnails(&d, 30);
        let mut left: Vec<String> =
            std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["making.1.3.tmp", "new.jpg", "newer.jpg", "old.jpg"]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_same_thumbnail_made_twice_at_once() {
        let d = std::env::temp_dir().join(format!("lituus-wallpaper-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        image::RgbImage::from_pixel(3000, 1700, image::Rgb([10, 120, 200])).save(d.join("a.png")).unwrap();
        for round in 0..20 {
            let cache = d.join(format!("cache{round}"));
            let threads: Vec<_> = (0..4)
                .map(|_| {
                    let (p, c) = (d.join("a.png"), cache.clone());
                    std::thread::spawn(move || thumbnail(&p, &c, 512, 288))
                })
                .collect();
            for t in threads {
                assert!(t.join().unwrap().is_ok(), "round {round}");
            }
            // Nothing left behind.
            assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1, "round {round}");
        }
        std::fs::remove_dir_all(&d).unwrap();
    }
}
