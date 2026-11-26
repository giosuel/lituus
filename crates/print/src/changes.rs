//! cupsd's own signals on the system bus (com.redhat.PrinterSpooler: a printer added or removed, a
//! queue or a printer's state changed, a job queued or started), sent without a subscription
//! whenever cupsd is built with D-Bus. Each only says "something changed": read again.

use futures_util::stream::{Stream, StreamExt};
use zbus::message::Type as MessageType;
use zbus::{Connection, MatchRule, MessageStream};

use crate::Error;

pub const INTERFACE: &str = "com.redhat.PrinterSpooler";

pub async fn changes(conn: &Connection) -> Result<impl Stream<Item = ()> + Unpin + use<>, Error> {
    let rule = MatchRule::builder().msg_type(MessageType::Signal).interface(INTERFACE)?.build();
    let stream = MessageStream::for_match_rule(rule, conn, Some(64)).await?;
    Ok(stream.filter_map(|m| async move { m.ok().map(drop) }).boxed())
}
