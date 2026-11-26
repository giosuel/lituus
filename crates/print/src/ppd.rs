//! A printer's PPD file (CUPS 2): apps (GTK, Qt, LibreOffice) take their defaults from it rather
//! than from the IPP `*-default` attributes, so a new default is written in both. Only the choices
//! the PPD has are written; a vendor's own option names are left alone.

use crate::media;

/// A PPD option with its default and choices: `*DefaultDuplex: None`, `*Duplex DuplexNoTumble/…: …`.
fn choices(ppd: &str, key: &str) -> Vec<String> {
    let prefix = format!("*{key} ");
    ppd.lines()
        .filter_map(|l| l.strip_prefix(&prefix))
        .filter_map(|rest| rest.split([':', '/']).next())
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

pub fn default(ppd: &str, key: &str) -> Option<String> {
    let prefix = format!("*Default{key}:");
    ppd.lines().find_map(|l| l.strip_prefix(&prefix)).map(|v| v.trim().to_string())
}

/// The PageSize choice the size of `media` (a PWG name) is, preferring one without a margin
/// variant (".Borderless", ".Fullbleed").
pub fn page_size(ppd: &str, media: &str) -> Option<String> {
    let (w, h) = media::size(media)?;
    let mut found: Vec<String> = ppd
        .lines()
        .filter_map(|l| l.strip_prefix("*PaperDimension "))
        .filter_map(|rest| {
            let (name, dims) = rest.split_once(':')?;
            let name = name.split('/').next()?.trim();
            let mut nums = dims.trim().trim_matches('"').split_whitespace().filter_map(|n| n.parse::<f64>().ok());
            let (pw, ph) = (nums.next()?, nums.next()?);
            ((pw - w).abs() < 2.0 && (ph - h).abs() < 2.0).then(|| name.to_string())
        })
        .collect();
    found.sort_by_key(|n| n.contains('.'));
    found.into_iter().next()
}

/// The PPD's (option, choice) pairs for new IPP defaults, where the PPD has that choice.
pub fn choices_for(
    ppd: &str,
    media: Option<&str>,
    sides: Option<&str>,
    color: Option<&str>,
    quality: Option<&str>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut add = |key: &str, choice: Option<&str>| {
        if let Some(c) = choice
            && choices(ppd, key).iter().any(|have| have == c)
        {
            out.push((key.to_string(), c.to_string()));
        }
    };
    if let Some(m) = media {
        let size = page_size(ppd, m);
        add("PageSize", size.as_deref());
    }
    add(
        "Duplex",
        sides.and_then(|s| match s {
            "one-sided" => Some("None"),
            "two-sided-long-edge" => Some("DuplexNoTumble"),
            "two-sided-short-edge" => Some("DuplexTumble"),
            _ => None,
        }),
    );
    if let Some(c) = color {
        let wanted: &[&str] = match c {
            "monochrome" | "process-monochrome" | "auto-monochrome" => &["Gray", "Grayscale", "KGray"],
            "color" | "auto" => &["RGB", "CMYK", "Color"],
            _ => &[],
        };
        let have = choices(ppd, "ColorModel");
        add("ColorModel", wanted.iter().find(|w| have.iter().any(|h| h == *w)).copied());
    }
    add(
        "cupsPrintQuality",
        quality.and_then(|q| match q {
            "draft" => Some("Draft"),
            "normal" => Some("Normal"),
            "high" => Some("High"),
            _ => None,
        }),
    );
    out
}

/// The PPD with these defaults. A new PageSize also sets PageRegion, ImageableArea and
/// PaperDimension, as CUPS's own tools do.
pub fn with_defaults(ppd: &str, set: &[(String, String)]) -> String {
    let mut keys: Vec<(String, String)> = set.to_vec();
    if let Some((_, size)) = set.iter().find(|(k, _)| k == "PageSize") {
        for k in ["PageRegion", "ImageableArea", "PaperDimension"] {
            keys.push((k.to_string(), size.clone()));
        }
    }
    let mut out = String::with_capacity(ppd.len());
    for line in ppd.split_inclusive('\n') {
        let replaced = keys.iter().find_map(|(k, v)| {
            line.starts_with(&format!("*Default{k}:")).then(|| {
                let end = if line.ends_with("\r\n") {
                    "\r\n"
                } else if line.ends_with('\n') {
                    "\n"
                } else {
                    ""
                };
                format!("*Default{k}: {v}{end}")
            })
        });
        out.push_str(replaced.as_deref().unwrap_or(line));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PPD: &str = "*PPD-Adobe: \"4.3\"\n\
*OpenUI *PageSize: PickOne\n\
*DefaultPageSize: Letter\n\
*PageSize A4/A4: \"<</PageSize[595.275590551181 841.889763779528]>>setpagedevice\"\n\
*PageSize A6.Borderless: \"<</PageSize[297.637795275591 419.527559055118]>>setpagedevice\"\n\
*PageSize Letter: \"<</PageSize[612 792]>>setpagedevice\"\n\
*DefaultPageRegion: Letter\n\
*DefaultImageableArea: Letter\n\
*DefaultPaperDimension: Letter\n\
*PaperDimension A4/A4: \"595.275590551181 841.889763779528\"\n\
*PaperDimension A6.Borderless: \"297.637795275591 419.527559055118\"\n\
*PaperDimension A6: \"297.637795275591 419.527559055118\"\n\
*PaperDimension Letter: \"612 792\"\n\
*OpenUI *cupsPrintQuality: PickOne\n\
*DefaultcupsPrintQuality: Normal\n\
*cupsPrintQuality Draft: \"\"\n\
*cupsPrintQuality Normal: \"\"\n\
*cupsPrintQuality High: \"\"\n\
*ColorModel RGB: \"\"\n\
*ColorModel Gray: \"\"\n\
*DefaultColorModel: RGB\n\
*OpenUI *Duplex/2-Sided Printing: PickOne\n\
*DefaultDuplex: None\n\
*Duplex None/Off: \"\"\n\
*Duplex DuplexNoTumble/Long-Edge: \"\"\n\
*Duplex DuplexTumble/Short-Edge: \"\"\n";

    #[test]
    fn defaults_and_page_sizes_are_read() {
        assert_eq!(default(PPD, "PageSize").as_deref(), Some("Letter"));
        assert_eq!(default(PPD, "Duplex").as_deref(), Some("None"));
        assert_eq!(page_size(PPD, "iso_a4_210x297mm").as_deref(), Some("A4"));
        assert_eq!(page_size(PPD, "iso_a6_105x148mm").as_deref(), Some("A6"));
        assert_eq!(page_size(PPD, "na_legal_8.5x14in"), None);
    }

    #[test]
    fn ipp_defaults_become_the_ppds_choices_where_it_has_them() {
        let set =
            choices_for(PPD, Some("iso_a4_210x297mm"), Some("two-sided-long-edge"), Some("monochrome"), Some("high"));
        let want: Vec<(String, String)> =
            [("PageSize", "A4"), ("Duplex", "DuplexNoTumble"), ("ColorModel", "Gray"), ("cupsPrintQuality", "High")]
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect();
        assert_eq!(set, want);
        assert!(choices_for(PPD, Some("na_legal_8.5x14in"), None, Some("auto-monochrome-x"), None).is_empty());
    }

    #[test]
    fn a_new_page_size_moves_its_region_and_dimensions_too() {
        let out = with_defaults(PPD, &[("PageSize".into(), "A4".into()), ("Duplex".into(), "DuplexTumble".into())]);
        for k in ["PageSize", "PageRegion", "ImageableArea", "PaperDimension"] {
            assert_eq!(default(&out, k).as_deref(), Some("A4"), "{k}");
        }
        assert_eq!(default(&out, "Duplex").as_deref(), Some("DuplexTumble"));
        assert_eq!(default(&out, "ColorModel").as_deref(), Some("RGB"));
        assert_eq!(out.lines().count(), PPD.lines().count());
    }
}
