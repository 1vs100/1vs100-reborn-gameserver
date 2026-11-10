//! XSOR documents as a small tree; parse then `to_bytes` returns the posted bytes.

use std::{borrow::Cow, fmt::Display};

use quick_xml::events::Event;

/// One XSOR element: `<name type="...">...</name>`.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    name: String,
    /// Names and values as written (values escaped).
    attrs: Vec<(String, String)>,
    content: Content,
}

#[derive(Clone, Debug, PartialEq)]
enum Content {
    /// Escaped text.
    Text(String),
    Children(Vec<Node>),
    /// `<name ... />`.
    Empty,
    /// Bytes stored as the title posted them (TopN metadata).
    Raw(Vec<u8>),
}

impl Node {
    fn text(name: impl Into<String>, kind: Option<&str>, value: impl Display) -> Node {
        Node {
            name: name.into(),
            attrs: kind
                .map(|kind| ("type".to_owned(), kind.to_owned()))
                .into_iter()
                .collect(),
            content: Content::Text(quick_xml::escape::escape(value.to_string()).into_owned()),
        }
    }

    /// An untyped (string) value.
    pub fn string(name: impl Into<String>, value: impl Display) -> Node {
        Node::text(name, None, value)
    }

    pub fn int(name: impl Into<String>, value: impl Display) -> Node {
        Node::text(name, Some("integer"), value)
    }

    pub fn float(name: impl Into<String>, value: impl Display) -> Node {
        Node::text(name, Some("float"), value)
    }

    pub fn boolean(name: impl Into<String>, value: bool) -> Node {
        Node::text(name, Some("boolean"), value)
    }

    pub fn table(name: impl Into<String>, children: impl IntoIterator<Item = Node>) -> Node {
        Node {
            name: name.into(),
            attrs: vec![("type".into(), "table".into())],
            content: Content::Children(children.into_iter().collect()),
        }
    }

    /// An untyped element whose content is written as is.
    pub fn raw(name: impl Into<String>, xml: &[u8]) -> Node {
        Node {
            name: name.into(),
            attrs: Vec::new(),
            content: Content::Raw(xml.to_vec()),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The value of attribute `name`, unescaped.
    pub fn attr(&self, name: &str) -> Option<Cow<'_, str>> {
        let (_, value) = self.attrs.iter().find(|(key, _)| key == name)?;
        quick_xml::escape::unescape(value).ok()
    }

    /// The text of a value element, unescaped; `None` for tables.
    pub fn text_value(&self) -> Option<Cow<'_, str>> {
        match &self.content {
            Content::Text(text) => quick_xml::escape::unescape(text).ok(),
            Content::Empty => Some(Cow::Borrowed("")),
            Content::Children(children) if children.is_empty() => Some(Cow::Borrowed("")),
            Content::Children(_) | Content::Raw(_) => None,
        }
    }

    pub fn children(&self) -> &[Node] {
        match &self.content {
            Content::Children(children) => children,
            _ => &[],
        }
    }

    /// The first child called `name`.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children().iter().find(|child| child.name == name)
    }

    pub fn child_mut(&mut self, name: &str) -> Option<&mut Node> {
        match &mut self.content {
            Content::Children(children) => children.iter_mut().find(|child| child.name == name),
            _ => None,
        }
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    /// Visit this element and every element below it, children before their parent.
    pub fn visit_mut(&mut self, f: &mut impl FnMut(&mut Node)) {
        if let Content::Children(children) = &mut self.content {
            children.iter_mut().for_each(|child| child.visit_mut(f));
        }
        f(self);
    }

    /// Replace this element by its child when that is its only content and is called `name`.
    pub fn unwrap_only_child(&mut self, name: &str) -> bool {
        match &mut self.content {
            Content::Children(children) if children.len() == 1 && children[0].name == name => {
                *self = children.pop().unwrap();
                true
            }
            _ => false,
        }
    }

    /// Replace the value of a value element (escaping `text`); false for a table with children.
    pub fn set_text(&mut self, text: impl Display) -> bool {
        if self.text_value().is_none() {
            return false;
        }
        self.content = Content::Text(quick_xml::escape::escape(text.to_string()).into_owned());
        true
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.push(b'<');
        out.extend_from_slice(self.name.as_bytes());
        for (key, value) in &self.attrs {
            out.extend_from_slice(format!(" {key}=\"{value}\"").as_bytes());
        }
        match &self.content {
            Content::Empty => return out.extend_from_slice(b" />"),
            Content::Text(text) => {
                out.push(b'>');
                out.extend_from_slice(text.as_bytes());
            }
            Content::Children(children) => {
                out.push(b'>');
                children.iter().for_each(|child| child.write(out));
            }
            Content::Raw(xml) => {
                out.push(b'>');
                out.extend_from_slice(xml);
            }
        }
        out.extend_from_slice(format!("</{}>", self.name).as_bytes());
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write(&mut out);
        out
    }
}

/// The element name of a list entry: a list is a table of `XsorItem`s.
pub const ITEM: &str = "XsorItem";

/// The server's own document form: `<xsor:object ... name="name" type="table">`.
pub fn object(name: &str, fields: impl IntoIterator<Item = Node>) -> Vec<u8> {
    Node {
        name: "xsor:object".into(),
        attrs: [
            ("xmlns:xsor", "http://schemas.xbox.com/xsor"),
            ("xmlns", "http://schemas.xbox.com/SBG"),
            ("name", name),
            ("type", "table"),
        ]
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .into(),
        content: Content::Children(fields.into_iter().collect()),
    }
    .to_bytes()
}

/// The form the title posts: `<XsorObject>...</XsorObject>`, `<XsorObject />` when empty.
pub fn native(fields: impl IntoIterator<Item = Node>) -> Vec<u8> {
    let fields: Vec<Node> = fields.into_iter().collect();
    Node {
        name: "XsorObject".into(),
        attrs: Vec::new(),
        content: if fields.is_empty() {
            Content::Empty
        } else {
            Content::Children(fields)
        },
    }
    .to_bytes()
}

/// Root element of `xml`; `None` unless well-formed with text or child elements.
pub fn parse(xml: &[u8]) -> Option<Node> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut open: Vec<(Node, String)> = Vec::new();
    let mut root = None;
    loop {
        let finished = match reader.read_event().ok()? {
            Event::Start(tag) => {
                open.push((element(&tag)?, String::new()));
                None
            }
            Event::Empty(tag) => Some(Node {
                content: Content::Empty,
                ..element(&tag)?
            }),
            Event::Text(text) => {
                let text = std::str::from_utf8(text.as_ref()).ok()?;
                if !text.trim().is_empty() {
                    open.last_mut()?.1.push_str(text);
                }
                None
            }
            Event::GeneralRef(entity) => {
                let entity = std::str::from_utf8(entity.as_ref()).ok()?;
                open.last_mut()?.1.push_str(&format!("&{entity};"));
                None
            }
            Event::End(_) => {
                let (mut node, text) = open.pop()?;
                match (&node.content, text.is_empty()) {
                    (Content::Children(children), false) if children.is_empty() => {
                        node.content = Content::Text(text)
                    }
                    (_, false) => return None,
                    _ => {}
                }
                Some(node)
            }
            Event::CData(_) | Event::DocType(_) => return None,
            Event::Eof => break,
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => None,
        };
        if let Some(node) = finished {
            match open.last_mut() {
                Some((parent, text)) if text.is_empty() => {
                    let Content::Children(children) = &mut parent.content else {
                        unreachable!("open elements hold children")
                    };
                    children.push(node);
                }
                Some(_) => return None,
                None if root.is_none() => root = Some(node),
                None => return None,
            }
        }
    }
    root.filter(|_| open.is_empty())
}

fn element(tag: &quick_xml::events::BytesStart) -> Option<Node> {
    let name = std::str::from_utf8(tag.name().as_ref()).ok()?.to_owned();
    let attrs = tag
        .attributes()
        .map(|attr| {
            let attr = attr.ok()?;
            Some((
                std::str::from_utf8(attr.key.as_ref()).ok()?.to_owned(),
                std::str::from_utf8(&attr.value).ok()?.to_owned(),
            ))
        })
        .collect::<Option<_>>()?;
    Some(Node {
        name,
        attrs,
        content: Content::Children(Vec::new()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_write_their_type_and_escape_text() {
        let xml = object(
            "question",
            [
                Node::string("Question", "Who's <first> & last?"),
                Node::int("CorrectAnswer", 2),
                Node::table(
                    "ChoiceList",
                    [Node::string(ITEM, "a"), Node::float(ITEM, 1.5)],
                ),
                Node::table("Empty", []),
                Node::raw("Metadata", b"<x>kept</x>"),
            ],
        );
        assert_eq!(
            String::from_utf8(xml).unwrap(),
            "<xsor:object xmlns:xsor=\"http://schemas.xbox.com/xsor\" xmlns=\"http://schemas.xbox.com/SBG\" name=\"question\" type=\"table\">\
             <Question>Who&apos;s &lt;first&gt; &amp; last?</Question>\
             <CorrectAnswer type=\"integer\">2</CorrectAnswer>\
             <ChoiceList type=\"table\"><XsorItem>a</XsorItem><XsorItem type=\"float\">1.5</XsorItem></ChoiceList>\
             <Empty type=\"table\"></Empty>\
             <Metadata><x>kept</x></Metadata>\
             </xsor:object>"
        );
    }

    /// The tree the parser reads, in the shapes a document holds: typed values, tables, a
    /// self-closed element, attributes, escaped text and raw metadata.
    #[test]
    fn documents_parse_to_their_nodes() {
        let parsed: [Option<Node>; 2] = [
            &b"<XsorObject name=\"question\" type=\"table\">\
              <Question>it&apos;s &lt;1&gt;</Question>\
              <CorrectAnswer type=\"integer\">2</CorrectAnswer>\
              <ChoiceList type=\"table\"><XsorItem>a</XsorItem>\
              <XsorItem subtype=\"x&amp;y\" type=\"float\">1.5</XsorItem></ChoiceList>\
              <Empty type=\"table\"></Empty><SelfClosed />\
              <Metadata><x>kept</x></Metadata></XsorObject>"[..],
            include_bytes!("../../fixtures/question-251317-en-us.xml").as_slice(),
        ]
        .map(parse);
        insta::assert_debug_snapshot!(parsed);
    }

    #[test]
    fn an_empty_native_document_is_self_closed() {
        assert_eq!(native([]), b"<XsorObject />");
        assert_eq!(
            native([Node::boolean("b", true)]),
            b"<XsorObject><b type=\"boolean\">true</b></XsorObject>"
        );
    }

    #[test]
    fn reading_keeps_what_was_written() {
        let xml = b"<XsorObject><a type=\"table\" /><b>it&apos;s &lt;1&gt;</b><c type=\"table\"><d subtype=\"x&amp;y\">1</d></c><e></e></XsorObject>";
        let root = parse(xml).unwrap();
        assert_eq!(root.to_bytes(), xml);
        assert_eq!(root.child("b").unwrap().text_value().unwrap(), "it's <1>");
        let d = &root.child("c").unwrap().children()[0];
        assert_eq!(d.attr("subtype").unwrap(), "x&y");
        assert_eq!(d.text_value().unwrap(), "1");
        assert_eq!(root.child("e").unwrap().text_value().unwrap(), "");
        assert_eq!(root.child("c").unwrap().text_value(), None);

        let mut root = root;
        assert!(root.child_mut("b").unwrap().set_text("a&b"));
        assert!(root.child_mut("a").unwrap().set_text(2));
        assert!(!root.child_mut("c").unwrap().set_text(3));
        assert_eq!(
            String::from_utf8(root.to_bytes()).unwrap(),
            "<XsorObject><a type=\"table\">2</a><b>a&amp;b</b><c type=\"table\"><d subtype=\"x&amp;y\">1</d></c><e></e></XsorObject>"
        );
    }

    #[test]
    fn only_one_well_formed_element_is_read() {
        for bad in [
            &b""[..],
            b"<a>",
            b"<a></b>",
            b"<a/><b/>",
            b"<a>text<b/></a>",
            b"<a><b/>text</a>",
            b"<a><![CDATA[x]]></a>",
            b"text",
        ] {
            assert_eq!(parse(bad), None, "{}", String::from_utf8_lossy(bad));
        }
        assert!(parse(b"<?xml version=\"1.0\"?>\n<a>\n  <b>1</b>\n</a>").is_some());
    }
}
