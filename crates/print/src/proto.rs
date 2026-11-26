//! IPP messages through the `ipp` crate's codec: building requests and reading attributes back.
//! Where an attribute repeats in a group (cupsd sends `media-default` twice when the IPP default and
//! the PPD's disagree), the last one counts: the PPD's, which apps follow.

use std::io::Cursor;

use ipp::model::{DelimiterTag, IppVersion, Operation};
use ipp::parser::IppParser;
use ipp::prelude::{IppAttribute, IppRequestResponse, IppValue};

use crate::Error;

pub(crate) struct Request {
    msg: IppRequestResponse,
}

fn attr(name: &str, value: IppValue) -> IppAttribute {
    IppAttribute::with_name(name, value).expect("attribute names are short")
}

/// A text value; anything over IPP's lengths is cut rather than refused.
pub(crate) fn name(s: &str) -> IppValue {
    IppValue::new_name_without_language(s)
        .unwrap_or_else(|_| IppValue::new_name_without_language(cut(s, 255)).expect("cut"))
}

#[cfg(test)]
pub(crate) fn text(s: &str) -> IppValue {
    IppValue::new_text_without_language(s)
        .unwrap_or_else(|_| IppValue::new_text_without_language(cut(s, 1023)).expect("cut"))
}

pub(crate) fn keyword(s: &str) -> IppValue {
    IppValue::new_keyword(s).unwrap_or_else(|_| IppValue::new_keyword(cut(s, 255)).expect("cut"))
}

pub(crate) fn uri(s: &str) -> IppValue {
    IppValue::new_uri(s).unwrap_or_else(|_| IppValue::new_uri(cut(s, 1023)).expect("cut"))
}

fn cut(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

impl Request {
    /// An operation with its charset and language, the printer's URI and the user.
    pub fn new(op: Operation, printer_uri: Option<&str>, user: &str) -> Self {
        let mut msg = IppRequestResponse::new(IppVersion::v2_0(), op, None).expect("no URI to check");
        if let Some(u) = printer_uri {
            msg.attributes_mut().add(DelimiterTag::OperationAttributes, attr("printer-uri", uri(u)));
        }
        msg.attributes_mut().add(DelimiterTag::OperationAttributes, attr("requesting-user-name", name(user)));
        Self { msg }
    }

    pub fn op(mut self, n: &str, value: IppValue) -> Self {
        self.msg.attributes_mut().add(DelimiterTag::OperationAttributes, attr(n, value));
        self
    }

    pub fn wanted(self, names: &[&str]) -> Self {
        let list = names.iter().map(|n| keyword(n)).collect();
        self.op("requested-attributes", IppValue::Array(list))
    }

    /// The message, then `document` (a PPD, a test page).
    pub fn bytes(&self, document: &[u8]) -> Vec<u8> {
        let mut out = self.msg.to_bytes().to_vec();
        out.extend_from_slice(document);
        out
    }
}

/// One group of a response: a printer, a job, the operation's own attributes.
#[derive(Debug, Clone)]
pub(crate) struct Group {
    pub tag: DelimiterTag,
    attrs: Vec<IppAttribute>,
}

fn string(v: &IppValue) -> Option<String> {
    match v {
        IppValue::TextWithoutLanguage(_)
        | IppValue::NameWithoutLanguage(_)
        | IppValue::Keyword(_)
        | IppValue::Uri(_)
        | IppValue::UriScheme(_)
        | IppValue::MimeMediaType(_)
        | IppValue::Charset(_)
        | IppValue::NaturalLanguage(_) => Some(v.to_string()),
        IppValue::TextWithLanguage { text, .. } => Some(text.to_string()),
        IppValue::NameWithLanguage { name, .. } => Some(name.to_string()),
        IppValue::NonUtf8 { data, .. } => Some(String::from_utf8_lossy(data).into_owned()),
        _ => None,
    }
}

fn int(v: &IppValue) -> Option<i32> {
    match v {
        IppValue::Integer(i) | IppValue::Enum(i) => Some(*i),
        _ => None,
    }
}

impl Group {
    fn last(&self, n: &str) -> Option<&IppValue> {
        self.attrs.iter().rev().find(|a| a.name().as_str() == n).map(|a| a.value())
    }

    /// One value; of a list, its first.
    fn one(&self, n: &str) -> Option<&IppValue> {
        match self.last(n)? {
            IppValue::Array(list) => list.first(),
            v => Some(v),
        }
    }

    fn all(&self, n: &str) -> Vec<&IppValue> {
        match self.last(n) {
            Some(IppValue::Array(list)) => list.iter().collect(),
            Some(v) => vec![v],
            None => vec![],
        }
    }

    pub fn text(&self, n: &str) -> Option<String> {
        self.one(n).and_then(string)
    }

    pub fn int(&self, n: &str) -> Option<i32> {
        self.one(n).and_then(int)
    }

    pub fn bool(&self, n: &str) -> Option<bool> {
        match self.one(n)? {
            IppValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn texts(&self, n: &str) -> Vec<String> {
        self.all(n).into_iter().filter_map(string).collect()
    }

    pub fn ints(&self, n: &str) -> Vec<i32> {
        self.all(n).into_iter().filter_map(int).collect()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Answer {
    /// IPP's status: below 0x0100 it worked.
    pub status: u16,
    pub groups: Vec<Group>,
}

impl Answer {
    pub fn ok(&self) -> bool {
        self.status < 0x0100
    }

    pub fn message(&self) -> String {
        self.of(DelimiterTag::OperationAttributes).find_map(|g| g.text("status-message")).unwrap_or_default()
    }

    pub fn of(&self, tag: DelimiterTag) -> impl Iterator<Item = &Group> {
        self.groups.iter().filter(move |g| g.tag == tag)
    }

    /// The answer, or the error it reports: 0x0401 to 0x0403 are "not allowed".
    pub fn checked(self) -> Result<Self, Error> {
        if self.ok() {
            return Ok(self);
        }
        let message = self.message();
        Err(match self.status {
            0x0401..=0x0403 => Error::NotAllowed(message),
            status => Error::Ipp { status, message },
        })
    }
}

pub(crate) fn parse(body: &[u8]) -> Result<Answer, Error> {
    let msg = IppParser::new(Cursor::new(body.to_vec())).parse().map_err(|e| Error::Parse(e.to_string()))?;
    let status = msg.header().operation_or_status as u16;
    let groups =
        msg.attributes().groups().iter().map(|g| Group { tag: g.tag(), attrs: g.attributes().to_vec() }).collect();
    Ok(Answer { status, groups })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A response as cupsd writes it, made with the same codec.
    pub(crate) fn response(status: u16, groups: Vec<(DelimiterTag, Vec<(&str, IppValue)>)>) -> Vec<u8> {
        let mut msg =
            IppRequestResponse::new_response(IppVersion::v2_0(), ipp::model::StatusCode::SuccessfulOk, 1).unwrap();
        msg.header_mut().operation_or_status = status as i16;
        for (tag, attrs) in groups {
            // The codec writes one operation group: its own, with the charset first.
            if tag == DelimiterTag::OperationAttributes {
                for (n, v) in attrs {
                    msg.attributes_mut().add(tag, attr(n, v));
                }
                continue;
            }
            let mut g = ipp::attribute::IppAttributeGroup::new(tag);
            for (n, v) in attrs {
                g.attributes_mut().push(attr(n, v));
            }
            msg.attributes_mut().groups_mut().push(g);
        }
        msg.to_bytes().to_vec()
    }

    #[test]
    fn a_request_has_the_printer_user_and_wanted_attributes() {
        let r = Request::new(Operation::GetPrinterAttributes, Some("ipp://localhost/printers/Office"), "anna")
            .wanted(&["media-default", "sides-default"]);
        let back = parse(&r.bytes(b"")).unwrap();
        let op = back.groups.iter().find(|g| g.tag == DelimiterTag::OperationAttributes).unwrap();
        assert_eq!(back.status, Operation::GetPrinterAttributes as u16);
        assert_eq!(op.text("printer-uri").as_deref(), Some("ipp://localhost/printers/Office"));
        assert_eq!(op.text("requesting-user-name").as_deref(), Some("anna"));
        assert_eq!(op.texts("requested-attributes"), ["media-default", "sides-default"]);
        assert_eq!(op.text("attributes-charset").as_deref(), Some("utf-8"));
    }

    #[test]
    fn a_repeated_attribute_counts_by_its_last_value() {
        let body = response(
            0,
            vec![(
                DelimiterTag::PrinterAttributes,
                vec![
                    ("media-default", keyword("na_letter_8.5x11in")),
                    (
                        "media-supported",
                        IppValue::Array(vec![keyword("iso_a4_210x297mm"), keyword("na_letter_8.5x11in")]),
                    ),
                    ("media-default", keyword("iso_a4_210x297mm")),
                    ("print-quality-supported", IppValue::Array(vec![IppValue::Enum(3), IppValue::Enum(4)])),
                    ("color-supported", IppValue::Boolean(false)),
                ],
            )],
        );
        let a = parse(&body).unwrap();
        let p = a.of(DelimiterTag::PrinterAttributes).next().unwrap();
        assert_eq!(p.text("media-default").as_deref(), Some("iso_a4_210x297mm"));
        assert_eq!(p.texts("media-supported").len(), 2);
        assert_eq!(p.ints("print-quality-supported"), [3, 4]);
        assert_eq!(p.bool("color-supported"), Some(false));
        assert_eq!(p.text("nothing"), None);
    }

    #[test]
    fn refusals_are_not_allowed_and_other_failures_keep_their_status() {
        let refused =
            response(0x0401, vec![(DelimiterTag::OperationAttributes, vec![("status-message", text("Forbidden"))])]);
        assert!(matches!(parse(&refused).unwrap().checked(), Err(Error::NotAllowed(m)) if m == "Forbidden"));
        let missing = response(0x0406, vec![]);
        assert!(matches!(parse(&missing).unwrap().checked(), Err(Error::Ipp { status: 0x0406, .. })));
        assert!(parse(&response(0, vec![])).unwrap().checked().is_ok());
    }
}
