//! USB printers that need a driver: their IEEE 1284 device ID ("MFG:Samsung;MDL:ML-2160;CMD:SPL;"),
//! and which package has a driver for the common ones. Lituus never installs drivers; the Add
//! dialog says which to install, or links OpenPrinting's list.

use std::collections::HashMap;

pub const OPENPRINTING: &str = "https://www.openprinting.org/printers";

/// The device ID's fields, by their short keys (MFG, MDL, CMD, CLS, DES).
pub fn device_id(id: &str) -> HashMap<String, String> {
    id.split(';')
        .filter_map(|f| f.split_once(':'))
        .map(|(k, v)| {
            let k = k.trim().to_ascii_uppercase();
            let k = match k.as_str() {
                "MANUFACTURER" => "MFG".to_string(),
                "MODEL" => "MDL".to_string(),
                "COMMAND SET" => "CMD".to_string(),
                _ => k,
            };
            (k, v.trim().to_string())
        })
        .collect()
}

/// The package with a driver for it, for the makes most USB printers that need one are.
pub fn package(id: &HashMap<String, String>) -> Option<&'static str> {
    let make = id.get("MFG").map(|m| m.to_ascii_lowercase()).unwrap_or_default();
    let model = id.get("MDL").map(|m| m.to_ascii_uppercase()).unwrap_or_default();
    let cmd = id.get("CMD").map(|c| c.to_ascii_uppercase()).unwrap_or_default();
    if cmd.contains("SPL") || cmd.contains("QPDL") {
        Some("splix")
    } else if cmd.contains("ZJS") || cmd.contains("ZJSTREAM") {
        Some("foo2zjs")
    } else if make.starts_with("hp") || make.contains("hewlett") {
        Some("hplip")
    } else if make.starts_with("brother") && ["HL-", "DCP-", "MFC-"].iter().any(|p| model.starts_with(p)) {
        Some("brlaser")
    } else if make.starts_with("epson") || make.starts_with("canon") {
        Some("gutenprint")
    } else {
        None
    }
}

/// "Samsung ML-2160 Series" from the device ID, else the fallback.
pub fn title(id: &HashMap<String, String>, fallback: &str) -> String {
    match (id.get("MFG"), id.get("MDL")) {
        (Some(m), Some(d)) if d.to_lowercase().starts_with(&m.to_lowercase()) => d.clone(),
        (Some(m), Some(d)) => format!("{m} {d}"),
        _ => fallback.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_ids_name_the_driver_package() {
        let id = device_id("MFG:Samsung;CMD:SPL,FWV,PIC,BDN,EXT;MDL:ML-2160 Series;CLS:PRINTER;");
        assert_eq!(id.get("MDL").map(String::as_str), Some("ML-2160 Series"));
        assert_eq!(package(&id), Some("splix"));
        assert_eq!(title(&id, ""), "Samsung ML-2160 Series");
        assert_eq!(package(&device_id("MANUFACTURER:HP;MODEL:HP LaserJet 1020;COMMAND SET:ZJS;")), Some("foo2zjs"));
        assert_eq!(title(&device_id("MFG:HP;MDL:HP LaserJet 1020;"), ""), "HP LaserJet 1020");
        assert_eq!(package(&device_id("MFG:HP;MDL:DeskJet 2700;CMD:PCL3GUI;")), Some("hplip"));
        assert_eq!(package(&device_id("MFG:Brother;MDL:HL-L2300D series;CMD:PJL,PCL;")), Some("brlaser"));
        assert_eq!(package(&device_id("MFG:EPSON;MDL:XP-2100 Series;CMD:ESCPL2;")), Some("gutenprint"));
        assert_eq!(package(&device_id("MFG:Zebra;MDL:ZD420;")), None);
    }
}
