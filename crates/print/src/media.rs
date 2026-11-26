//! Paper sizes by their PWG names (PWG 5101.1: `iso_a4_210x297mm`, `na_letter_8.5x11in`): what to
//! call them, and how big they are.

const NAMES: &[(&str, &str)] = &[
    ("iso_a3", "A3"),
    ("iso_a4", "A4"),
    ("iso_a5", "A5"),
    ("iso_a6", "A6"),
    ("iso_b4", "B4"),
    ("iso_b5", "B5"),
    ("jis_b4", "B4 (JIS)"),
    ("jis_b5", "B5 (JIS)"),
    ("na_letter", "Letter"),
    ("na_legal", "Legal"),
    ("na_executive", "Executive"),
    ("na_ledger", "Tabloid"),
    ("na_invoice", "Statement"),
    ("na_foolscap", "Foolscap"),
    ("iso_dl", "Envelope DL"),
    ("iso_c4", "Envelope C4"),
    ("iso_c5", "Envelope C5"),
    ("iso_c6", "Envelope C6"),
    ("na_number-10", "Envelope #10"),
    ("na_monarch", "Envelope Monarch"),
    ("na_index-3x5", "3 × 5 in"),
    ("na_index-4x6", "4 × 6 in"),
    ("na_5x7", "5 × 7 in"),
    ("na_govt-letter", "8 × 10 in"),
    ("oe_photo-l", "3.5 × 5 in"),
    ("om_small-photo", "10 × 15 cm"),
];

/// Width and height in points, from the name's last part ("210x297mm", "8.5x11in").
pub fn size(keyword: &str) -> Option<(f64, f64)> {
    let dims = keyword.rsplit('_').next()?;
    let (dims, scale) = match dims.strip_suffix("mm") {
        Some(d) => (d, 72.0 / 25.4),
        None => (dims.strip_suffix("in")?, 72.0),
    };
    let (w, h) = dims.split_once('x')?;
    Some((w.parse::<f64>().ok()? * scale, h.parse::<f64>().ok()? * scale))
}

/// Sizes that aren't papers: a printer's smallest and largest custom size.
pub fn is_custom_range(keyword: &str) -> bool {
    keyword.starts_with("custom_min") || keyword.starts_with("custom_max")
}

fn number(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(String::from).unwrap_or(s)
}

/// "A4", "Letter", or its size: "210 × 297 mm".
pub fn name(keyword: &str) -> String {
    let class = keyword.rsplit_once('_').map(|(c, _)| c).unwrap_or(keyword);
    if let Some((_, n)) = NAMES.iter().find(|(k, _)| *k == class) {
        return n.to_string();
    }
    let dims = keyword.rsplit('_').next().unwrap_or(keyword);
    for unit in ["mm", "in"] {
        if let Some((w, h)) = dims.strip_suffix(unit).and_then(|d| d.split_once('x'))
            && let (Ok(w), Ok(h)) = (w.parse::<f64>(), h.parse::<f64>())
        {
            return format!("{} × {} {unit}", number(w), number(h));
        }
    }
    keyword.to_string()
}

/// The paper this machine's locale uses (LC_PAPER, from `locale`): A4, or Letter in North America.
pub fn local_paper(lc_paper: Option<&str>) -> &'static str {
    let l = lc_paper.unwrap_or("");
    let letter = ["_US", "_CA", "_MX", "_PH", "_CL", "_CO", "_VE", "_PR", "_GT", "_PA", "_SV", "_CR", "_NI", "_DO"];
    if letter.iter().any(|c| l.contains(c)) { "na_letter_8.5x11in" } else { "iso_a4_210x297mm" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn papers_have_their_names_or_their_sizes() {
        assert_eq!(name("iso_a4_210x297mm"), "A4");
        assert_eq!(name("na_letter_8.5x11in"), "Letter");
        assert_eq!(name("na_number-10_4.125x9.5in"), "Envelope #10");
        assert_eq!(name("om_123x456mm_123x456mm"), "123 × 456 mm");
        assert_eq!(name("na_custom_8.5x13.5in"), "8.5 × 13.5 in");
        assert_eq!(name("weird"), "weird");
        assert!(is_custom_range("custom_min_3x5in") && !is_custom_range("iso_a4_210x297mm"));
    }

    #[test]
    fn sizes_are_in_points() {
        let (w, h) = size("iso_a4_210x297mm").unwrap();
        assert!((w - 595.28).abs() < 0.01 && (h - 841.89).abs() < 0.01, "{w} {h}");
        assert_eq!(size("na_letter_8.5x11in"), Some((612.0, 792.0)));
        assert_eq!(size("nothing"), None);
    }

    #[test]
    fn north_america_prints_on_letter() {
        assert_eq!(local_paper(Some("en_US.UTF-8")), "na_letter_8.5x11in");
        assert_eq!(local_paper(Some("de_CH.UTF-8")), "iso_a4_210x297mm");
        assert_eq!(local_paper(None), "iso_a4_210x297mm");
    }
}
