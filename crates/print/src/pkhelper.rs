//! cups-pk-helper (on the system bus, as root): changes for users outside CUPS's admin group,
//! after polkit asks for the password, as GNOME Settings does it. Each call answers with an
//! error text, empty when it worked.

use zbus::Connection;
use zbus::proxy::MethodFlags;

use crate::Error;
use crate::model::Defaults;

pub const NAME: &str = "org.opensuse.CupsPkHelper.Mechanism";
const PATH: &str = "/";

/// Who may change printers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Access {
    /// One of CUPS's admins (SystemGroup): through the socket, no password.
    #[default]
    Admin,
    /// Through cups-pk-helper: polkit asks for the password.
    PkHelper,
    /// Neither: changes can't be made.
    None,
}

/// Whether cups-pk-helper is there (running, or started on the first call).
pub async fn available(conn: &Connection) -> bool {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else { return false };
    if let Ok(name) = NAME.try_into()
        && dbus.name_has_owner(name).await.unwrap_or(false)
    {
        return true;
    }
    dbus.list_activatable_names().await.map(|names| names.iter().any(|n| n.as_str() == NAME)).unwrap_or(false)
}

/// Admin by the user's groups; else cups-pk-helper if it's there.
pub async fn access(system: Option<&Connection>) -> Access {
    if crate::user::is_admin(&crate::user::groups()) {
        return Access::Admin;
    }
    match system {
        Some(conn) if available(conn).await => Access::PkHelper,
        _ => Access::None,
    }
}

fn refused(e: &zbus::Error) -> bool {
    let s = e.to_string();
    ["NotPrivileged", "AccessDenied", "AuthFailed", "NotAuthorized", "InteractiveAuthorizationRequired"]
        .iter()
        .any(|k| s.contains(k))
}

#[derive(Clone)]
pub struct PkHelper {
    conn: Connection,
}

impl PkHelper {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }

    async fn call<B>(&self, method: &str, body: &B) -> Result<(), Error>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let proxy = zbus::Proxy::new(&self.conn, NAME, PATH, NAME).await?;
        let answer: Option<String> = proxy
            .call_with_flags(method, MethodFlags::AllowInteractiveAuth.into(), body)
            .await
            .map_err(|e| if refused(&e) { Error::NotAllowed(String::new()) } else { Error::DBus(e) })?;
        match answer.unwrap_or_default() {
            e if e.is_empty() => Ok(()),
            e if e.to_lowercase().contains("not authorized") || e.to_lowercase().contains("forbidden") => {
                Err(Error::NotAllowed(e))
            }
            e => Err(Error::Ipp { status: 0, message: e }),
        }
    }

    pub async fn remove(&self, name: &str) -> Result<(), Error> {
        self.call("PrinterDelete", &(name,)).await
    }

    pub async fn cancel(&self, job: i32) -> Result<(), Error> {
        self.call("JobCancelPurge", &(job, false)).await
    }

    /// Adds a printer: `ppd` is a driver's name, or "everywhere" for a driverless one.
    pub async fn add(&self, name: &str, uri: &str, ppd: &str, info: &str, location: &str) -> Result<(), Error> {
        self.call("PrinterAdd", &(name, uri, ppd, info, location)).await
    }

    /// New defaults: the IPP option, and the PPD's (cups-pk-helper writes those into the PPD).
    pub async fn set_options(&self, name: &str, d: &Defaults, ppd_choices: &[(String, String)]) -> Result<(), Error> {
        let ipp =
            [("media", &d.media), ("sides", &d.sides), ("print-color-mode", &d.color), ("print-quality", &d.quality)];
        for (option, value) in ipp {
            if let Some(v) = value {
                let v = if option == "print-quality" {
                    crate::model::quality_value(v).map(|n| n.to_string()).unwrap_or_else(|| v.clone())
                } else {
                    v.clone()
                };
                self.call("PrinterAddOptionDefault", &(name, option, vec![v])).await?;
            }
        }
        for (key, choice) in ppd_choices {
            self.call("PrinterAddOptionDefault", &(name, key.as_str(), vec![choice.clone()])).await?;
        }
        Ok(())
    }
}
