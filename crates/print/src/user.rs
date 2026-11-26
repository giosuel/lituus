//! Who Lituus runs as: the name cupsd checks against the socket's peer (PeerCred), and the groups
//! that make someone a CUPS admin.

/// A field of /proc/self/status: "Uid:" or "Groups:".
fn status_ids(status: &str, key: &str) -> Vec<u32> {
    status
        .lines()
        .find_map(|l| l.strip_prefix(key))
        .map(|rest| rest.split_whitespace().filter_map(|n| n.parse().ok()).collect())
        .unwrap_or_default()
}

/// The name for `uid` in a passwd file.
fn passwd_name(passwd: &str, uid: u32) -> Option<String> {
    passwd.lines().find_map(|l| {
        let mut f = l.split(':');
        let name = f.next()?;
        let id: u32 = f.nth(1)?.parse().ok()?;
        (id == uid && !name.is_empty()).then(|| name.to_string())
    })
}

/// The names of `gids` in a group file.
fn group_names(group: &str, gids: &[u32]) -> Vec<String> {
    group
        .lines()
        .filter_map(|l| {
            let mut f = l.split(':');
            let name = f.next()?;
            let id: u32 = f.nth(1)?.parse().ok()?;
            gids.contains(&id).then(|| name.to_string())
        })
        .collect()
}

/// The user's name: from the real uid, as cupsd will see it, else $USER.
pub fn name() -> String {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let uid = status_ids(&status, "Uid:").first().copied();
    uid.and_then(|uid| passwd_name(&std::fs::read_to_string("/etc/passwd").unwrap_or_default(), uid))
        .or_else(|| std::env::var("USER").ok())
        .or_else(|| std::env::var("LOGNAME").ok())
        .or_else(|| uid.map(|u| u.to_string()))
        .unwrap_or_else(|| "root".into())
}

/// The names of the groups the process is in.
pub fn groups() -> Vec<String> {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let mut gids = status_ids(&status, "Groups:");
    gids.extend(status_ids(&status, "Gid:").first());
    group_names(&std::fs::read_to_string("/etc/group").unwrap_or_default(), &gids)
}

/// CUPS's SystemGroup on Arch and Fedora (sys, root, wheel) and Debian (lpadmin): members change
/// printers through the socket without a password.
pub fn is_admin(groups: &[String]) -> bool {
    groups.iter().any(|g| ["wheel", "sys", "root", "lpadmin"].contains(&g.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_and_groups_come_from_the_ids() {
        let status = "Name:\tlituus\nUid:\t1000\t1000\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\nGroups:\t10 998 \n";
        assert_eq!(status_ids(status, "Uid:"), [1000, 1000, 1000, 1000]);
        assert_eq!(status_ids(status, "Groups:"), [10, 998]);
        let passwd = "root:x:0:0::/root:/bin/bash\nanna:x:1000:1000::/home/anna:/usr/bin/fish\n";
        assert_eq!(passwd_name(passwd, 1000).as_deref(), Some("anna"));
        assert_eq!(passwd_name(passwd, 1001), None);
        let group = "root:x:0:root\nwheel:x:10:anna\nanna:x:1000:\nwheel2:x:9:\n";
        assert_eq!(group_names(group, &[10, 1000]), ["wheel", "anna"]);
        assert!(is_admin(&["anna".into(), "wheel".into()]));
        assert!(is_admin(&["lpadmin".into()]));
        assert!(!is_admin(&["anna".into(), "video".into()]));
    }
}
