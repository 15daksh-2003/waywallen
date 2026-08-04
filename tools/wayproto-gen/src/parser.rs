use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Debug)]
pub struct ParseError {
    pub pos: usize,
    pub msg: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parse error at byte {}: {}", self.pos, self.msg)
    }
}

impl std::error::Error for ParseError {}

fn err<T>(pos: usize, msg: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError {
        pos,
        msg: msg.into(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgType {
    Bool,
    U32,
    I32,
    U64,
    I64,
    F32,
    F64,
    String,
    Array(Box<ArgType>),
    KvList,
    Rect,
    Named(String),
    Enum(String),
}

impl ArgType {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "bool" => Self::Bool,
            "u32" => Self::U32,
            "i32" => Self::I32,
            "u64" => Self::U64,
            "i64" => Self::I64,
            "f32" => Self::F32,
            "f64" => Self::F64,
            "string" => Self::String,
            "kv_list" => Self::KvList,
            "rect" => Self::Rect,
            "array" => Self::Array(Box::new(Self::U32)), // placeholder, filled by element attr
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct NamedStruct {
    pub name: String,
    pub fields: Vec<Arg>,
}

#[derive(Debug, Clone)]
pub struct NamedEnum {
    pub name: String,
    pub entries: Vec<EnumEntry>,
}

#[derive(Debug, Clone)]
pub struct EnumEntry {
    pub name: String,
    pub value: u32,
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub name: String,
    pub ty: ArgType,
}

#[derive(Debug, Clone)]
pub enum FdSpec {
    None,
    Fixed(u32),
    /// Product of field names, e.g. "count * planes_per_buffer".
    Product(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct Message {
    pub name: String,
    pub opcode: u16,
    pub args: Vec<Arg>,
    pub fds: FdSpec,
}

/// Naming style for the daemon-to-peer direction.
/// `<request>` keeps Wayland-style enum and opcode names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundKind {
    Request,
    EventIn,
}

#[derive(Debug, Clone)]
pub struct Protocol {
    pub name: String,
    pub version: u32,
    pub inbound_kind: InboundKind,
    pub enums: Vec<NamedEnum>,
    pub structs: Vec<NamedStruct>,
    pub requests: Vec<Message>,
    pub events: Vec<Message>,
}

/// Minimal XML DOM node.
#[derive(Debug)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
}

impl Node {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

pub fn parse_protocol(src: &str) -> Result<Protocol, ParseError> {
    let mut p = Parser {
        src: src.as_bytes(),
        pos: 0,
    };
    p.skip_prolog()?;
    let root = p.parse_element()?;
    if root.name != "protocol" {
        return err(0, format!("root must be <protocol>, got <{}>", root.name));
    }
    let name = root
        .attr("name")
        .ok_or_else(|| ParseError {
            pos: 0,
            msg: "protocol missing name".into(),
        })?
        .to_string();
    let version: u32 = root
        .attr("version")
        .ok_or_else(|| ParseError {
            pos: 0,
            msg: "protocol missing version".into(),
        })?
        .parse()
        .map_err(|_| ParseError {
            pos: 0,
            msg: "protocol version not u32".into(),
        })?;

    let enum_nodes: Vec<&Node> = root
        .children
        .iter()
        .filter(|child| child.name == "enum")
        .collect();
    let mut enum_names = HashSet::new();
    for node in &enum_nodes {
        let enum_name = node.attr("name").ok_or_else(|| ParseError {
            pos: 0,
            msg: "<enum> missing name".into(),
        })?;
        if !enum_names.insert(enum_name.to_string()) {
            return err(0, format!("duplicate enum {enum_name}"));
        }
    }
    let enums = enum_nodes
        .into_iter()
        .map(parse_enum)
        .collect::<Result<Vec<_>, _>>()?;

    let struct_nodes: Vec<&Node> = root
        .children
        .iter()
        .filter(|child| child.name == "struct")
        .collect();
    let mut struct_names = HashSet::new();
    for node in &struct_nodes {
        let struct_name = node.attr("name").ok_or_else(|| ParseError {
            pos: 0,
            msg: "<struct> missing name".into(),
        })?;
        if !struct_names.insert(struct_name.to_string()) {
            return err(0, format!("duplicate struct {struct_name}"));
        }
        if enum_names.contains(struct_name) {
            return err(
                0,
                format!("type {struct_name} is both an enum and a struct"),
            );
        }
    }
    let parsed_structs = struct_nodes
        .into_iter()
        .map(|node| parse_struct(node, &struct_names, &enum_names))
        .collect::<Result<Vec<_>, _>>()?;
    let structs = order_structs(parsed_structs)?;

    let mut requests = Vec::new();
    let mut events = Vec::new();
    let mut inbound_kind: Option<InboundKind> = None;
    for child in &root.children {
        match child.name.as_str() {
            "request" => {
                set_inbound_kind(&mut inbound_kind, InboundKind::Request)?;
                requests.push(parse_message(child, &struct_names, &enum_names)?);
            }
            "event_in" => {
                set_inbound_kind(&mut inbound_kind, InboundKind::EventIn)?;
                requests.push(parse_message(child, &struct_names, &enum_names)?);
            }
            "event" => events.push(parse_message(child, &struct_names, &enum_names)?),
            "enum" | "struct" => {}
            other => return err(0, format!("unknown top-level element <{other}>")),
        }
    }

    validate_fd_paths(&requests, &structs)?;
    validate_fd_paths(&events, &structs)?;

    Ok(Protocol {
        name,
        version,
        inbound_kind: inbound_kind.unwrap_or(InboundKind::Request),
        enums,
        structs,
        requests,
        events,
    })
}

fn validate_fd_paths(messages: &[Message], structs: &[NamedStruct]) -> Result<(), ParseError> {
    let structs: HashMap<&str, &NamedStruct> = structs
        .iter()
        .map(|item| (item.name.as_str(), item))
        .collect();

    for message in messages {
        let FdSpec::Product(paths) = &message.fds else {
            continue;
        };
        for path in paths {
            let mut segments = path.split('.');
            let root = segments.next().expect("validated non-empty fd path");
            let mut ty = &message
                .args
                .iter()
                .find(|arg| arg.name == root)
                .ok_or_else(|| ParseError {
                    pos: 0,
                    msg: format!(
                        "count_expr path {path} does not start with a field of message {}",
                        message.name
                    ),
                })?
                .ty;

            for segment in segments {
                let ArgType::Named(struct_name) = ty else {
                    return err(
                        0,
                        format!("count_expr path {path} traverses non-struct field {segment}"),
                    );
                };
                let item = structs
                    .get(struct_name.as_str())
                    .expect("validated named struct reference");
                ty = &item
                    .fields
                    .iter()
                    .find(|field| field.name == segment)
                    .ok_or_else(|| ParseError {
                        pos: 0,
                        msg: format!(
                            "count_expr path {path} has no field {segment} in struct {struct_name}"
                        ),
                    })?
                    .ty;
            }

            if !matches!(ty, ArgType::U32 | ArgType::U64) {
                return err(
                    0,
                    format!("count_expr path {path} must resolve to u32 or u64"),
                );
            }
        }
    }
    Ok(())
}

fn parse_enum(node: &Node) -> Result<NamedEnum, ParseError> {
    let name = node
        .attr("name")
        .ok_or_else(|| ParseError {
            pos: 0,
            msg: "<enum> missing name".into(),
        })?
        .to_string();
    let mut entries = Vec::new();
    let mut entry_names = HashSet::new();
    let mut entry_values = HashSet::new();
    for child in &node.children {
        if child.name != "entry" {
            return err(0, format!("unknown enum child <{}>", child.name));
        }
        let entry_name = child.attr("name").ok_or_else(|| ParseError {
            pos: 0,
            msg: "<entry> missing name".into(),
        })?;
        if !entry_names.insert(entry_name.to_string()) {
            return err(0, format!("duplicate entry {entry_name} in enum {name}"));
        }
        let value = child
            .attr("value")
            .ok_or_else(|| ParseError {
                pos: 0,
                msg: "<entry> missing value".into(),
            })?
            .parse::<u32>()
            .map_err(|_| ParseError {
                pos: 0,
                msg: format!("entry {entry_name} value is not u32"),
            })?;
        if !entry_values.insert(value) {
            return err(0, format!("duplicate value {value} in enum {name}"));
        }
        entries.push(EnumEntry {
            name: entry_name.to_string(),
            value,
        });
    }
    if entries.is_empty() {
        return err(0, format!("enum {name} must contain at least one entry"));
    }
    Ok(NamedEnum { name, entries })
}

fn set_inbound_kind(slot: &mut Option<InboundKind>, k: InboundKind) -> Result<(), ParseError> {
    match *slot {
        Some(prev) if prev != k => err(
            0,
            "protocol mixes <request> and <event_in>; pick exactly one",
        ),
        _ => {
            *slot = Some(k);
            Ok(())
        }
    }
}

fn parse_struct(
    node: &Node,
    struct_names: &HashSet<String>,
    enum_names: &HashSet<String>,
) -> Result<NamedStruct, ParseError> {
    let name = node
        .attr("name")
        .ok_or_else(|| ParseError {
            pos: 0,
            msg: "<struct> missing name".into(),
        })?
        .to_string();
    let mut fields = Vec::new();
    let mut field_names = HashSet::new();
    for child in &node.children {
        if child.name != "field" {
            return err(0, format!("unknown struct child <{}>", child.name));
        }
        let field = parse_typed_field(child, struct_names, enum_names)?;
        if !field_names.insert(field.name.clone()) {
            return err(
                0,
                format!("duplicate field {} in struct {name}", field.name),
            );
        }
        if !matches!(
            field.ty,
            ArgType::Bool
                | ArgType::U32
                | ArgType::I32
                | ArgType::U64
                | ArgType::I64
                | ArgType::F32
                | ArgType::F64
                | ArgType::String
                | ArgType::Array(_)
                | ArgType::KvList
                | ArgType::Rect
                | ArgType::Named(_)
                | ArgType::Enum(_)
        ) {
            return err(
                0,
                format!("struct field {} uses unsupported owned type", field.name),
            );
        }
        fields.push(field);
    }
    if fields.is_empty() {
        return err(0, format!("struct {name} must contain at least one field"));
    }
    Ok(NamedStruct { name, fields })
}

fn order_structs(structs: Vec<NamedStruct>) -> Result<Vec<NamedStruct>, ParseError> {
    fn visit(
        name: &str,
        by_name: &HashMap<String, NamedStruct>,
        visiting: &mut HashSet<String>,
        visited: &mut HashSet<String>,
        out: &mut Vec<NamedStruct>,
    ) -> Result<(), ParseError> {
        if visited.contains(name) {
            return Ok(());
        }
        if !visiting.insert(name.to_string()) {
            return err(0, format!("recursive struct reference involving {name}"));
        }
        let item = by_name.get(name).expect("validated struct name");
        for field in &item.fields {
            if let ArgType::Named(dependency) = &field.ty {
                visit(dependency, by_name, visiting, visited, out)?;
            }
        }
        visiting.remove(name);
        visited.insert(name.to_string());
        out.push(item.clone());
        Ok(())
    }

    let by_name: HashMap<String, NamedStruct> = structs
        .into_iter()
        .map(|item| (item.name.clone(), item))
        .collect();
    let mut names: Vec<String> = by_name.keys().cloned().collect();
    names.sort();
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        visit(&name, &by_name, &mut visiting, &mut visited, &mut out)?;
    }
    Ok(out)
}

fn parse_message(
    node: &Node,
    struct_names: &HashSet<String>,
    enum_names: &HashSet<String>,
) -> Result<Message, ParseError> {
    let name = node
        .attr("name")
        .ok_or_else(|| ParseError {
            pos: 0,
            msg: format!("<{}> missing name", node.name),
        })?
        .to_string();
    let opcode: u16 = node
        .attr("opcode")
        .ok_or_else(|| ParseError {
            pos: 0,
            msg: format!("<{}> missing opcode", node.name),
        })?
        .parse()
        .map_err(|_| ParseError {
            pos: 0,
            msg: "opcode not u16".into(),
        })?;

    let mut args = Vec::new();
    let mut fds = FdSpec::None;
    for child in &node.children {
        match child.name.as_str() {
            "arg" => args.push(parse_typed_field(child, struct_names, enum_names)?),
            "fds" => fds = parse_fds(child)?,
            other => return err(0, format!("unknown message child <{other}>")),
        }
    }
    Ok(Message {
        name,
        opcode,
        args,
        fds,
    })
}

fn parse_typed_field(
    node: &Node,
    struct_names: &HashSet<String>,
    enum_names: &HashSet<String>,
) -> Result<Arg, ParseError> {
    let name = node
        .attr("name")
        .ok_or_else(|| ParseError {
            pos: 0,
            msg: "<arg> missing name".into(),
        })?
        .to_string();
    let ty_str = node.attr("type").ok_or_else(|| ParseError {
        pos: 0,
        msg: "<arg> missing type".into(),
    })?;
    let ty = if ty_str == "array" {
        let elem_str = node.attr("element").ok_or_else(|| ParseError {
            pos: 0,
            msg: "<arg type=\"array\"> missing element".into(),
        })?;
        let elem = ArgType::parse(elem_str).ok_or_else(|| ParseError {
            pos: 0,
            msg: format!("unknown array element type {elem_str}"),
        })?;
        match elem {
            ArgType::Bool | ArgType::Array(_) | ArgType::KvList => {
                return err(0, "arrays of bool / arrays / kv_list not supported");
            }
            _ => ArgType::Array(Box::new(elem)),
        }
    } else {
        match ArgType::parse(ty_str) {
            Some(ty) => ty,
            None if struct_names.contains(ty_str) => ArgType::Named(ty_str.to_string()),
            None if enum_names.contains(ty_str) => ArgType::Enum(ty_str.to_string()),
            None => return err(0, format!("unknown type {ty_str}")),
        }
    };
    Ok(Arg { name, ty })
}

fn parse_fds(node: &Node) -> Result<FdSpec, ParseError> {
    if let Some(s) = node.attr("count") {
        let n: u32 = s.parse().map_err(|_| ParseError {
            pos: 0,
            msg: "fds count not u32".into(),
        })?;
        if n == 0 {
            Ok(FdSpec::None)
        } else {
            Ok(FdSpec::Fixed(n))
        }
    } else if let Some(expr) = node.attr("count_expr") {
        // Support only products of message fields or nested struct fields.
        // Each operand is an identifier path such as `pool.count`.
        let parts: Vec<String> = expr
            .split('*')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        if parts.is_empty() {
            return err(0, "empty count_expr");
        }
        for p in &parts {
            if p.split('.').any(|segment| {
                segment.is_empty()
                    || !segment
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
            }) {
                return err(0, format!("count_expr field not an ident path: {p}"));
            }
        }
        Ok(FdSpec::Product(parts))
    } else {
        err(0, "<fds> missing count or count_expr")
    }
}

// ---------- Tokenizer / tree builder ----------

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn eof(&self) -> bool {
        self.pos >= self.src.len()
    }
    fn peek(&self) -> u8 {
        self.src[self.pos]
    }
    fn starts_with(&self, needle: &[u8]) -> bool {
        self.src[self.pos..].starts_with(needle)
    }
    fn advance(&mut self, n: usize) {
        self.pos += n;
    }
    fn skip_whitespace(&mut self) {
        while !self.eof() && self.peek().is_ascii_whitespace() {
            self.pos += 1;
        }
    }
    fn skip_comments_and_ws(&mut self) -> Result<(), ParseError> {
        loop {
            self.skip_whitespace();
            if self.eof() {
                return Ok(());
            }
            if self.starts_with(b"<!--") {
                self.advance(4);
                while !self.eof() && !self.starts_with(b"-->") {
                    self.advance(1);
                }
                if self.eof() {
                    return err(self.pos, "unterminated comment");
                }
                self.advance(3);
                continue;
            }
            return Ok(());
        }
    }
    fn skip_prolog(&mut self) -> Result<(), ParseError> {
        self.skip_comments_and_ws()?;
        if self.starts_with(b"<?xml") {
            while !self.eof() && !self.starts_with(b"?>") {
                self.advance(1);
            }
            if self.eof() {
                return err(self.pos, "unterminated xml decl");
            }
            self.advance(2);
        }
        self.skip_comments_and_ws()?;
        Ok(())
    }

    fn parse_element(&mut self) -> Result<Node, ParseError> {
        self.skip_comments_and_ws()?;
        if self.eof() || self.peek() != b'<' {
            return err(self.pos, "expected element");
        }
        self.advance(1);
        let name = self.parse_name()?;
        let mut attrs = Vec::new();
        loop {
            self.skip_whitespace();
            if self.eof() {
                return err(self.pos, "unterminated tag");
            }
            if self.peek() == b'/' {
                self.advance(1);
                if self.eof() || self.peek() != b'>' {
                    return err(self.pos, "expected '>' after '/'");
                }
                self.advance(1);
                return Ok(Node {
                    name,
                    attrs,
                    children: Vec::new(),
                });
            }
            if self.peek() == b'>' {
                self.advance(1);
                break;
            }
            let k = self.parse_name()?;
            self.skip_whitespace();
            if self.eof() || self.peek() != b'=' {
                return err(self.pos, "expected '='");
            }
            self.advance(1);
            self.skip_whitespace();
            let v = self.parse_attr_value()?;
            attrs.push((k, v));
        }

        let mut children = Vec::new();
        loop {
            self.skip_comments_and_ws()?;
            if self.starts_with(b"</") {
                self.advance(2);
                let close = self.parse_name()?;
                if close != name {
                    return err(self.pos, format!("mismatched close </{close}> vs <{name}>"));
                }
                self.skip_whitespace();
                if self.eof() || self.peek() != b'>' {
                    return err(self.pos, "expected '>' on close tag");
                }
                self.advance(1);
                return Ok(Node {
                    name,
                    attrs,
                    children,
                });
            }
            children.push(self.parse_element()?);
        }
    }

    fn parse_name(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        while !self.eof() {
            let c = self.peek();
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b':' {
                self.advance(1);
            } else {
                break;
            }
        }
        if start == self.pos {
            return err(self.pos, "expected name");
        }
        Ok(String::from_utf8_lossy(&self.src[start..self.pos]).into_owned())
    }

    fn parse_attr_value(&mut self) -> Result<String, ParseError> {
        if self.eof() {
            return err(self.pos, "expected attribute value");
        }
        let quote = self.peek();
        if quote != b'"' && quote != b'\'' {
            return err(self.pos, "expected quote");
        }
        self.advance(1);
        let start = self.pos;
        while !self.eof() && self.peek() != quote {
            self.advance(1);
        }
        if self.eof() {
            return err(self.pos, "unterminated attribute");
        }
        let v = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
        self.advance(1);
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_trivial() {
        let src = r#"<?xml version="1.0"?>
            <protocol name="foo" version="1">
                <request name="hello" opcode="1">
                    <arg name="x" type="u32"/>
                </request>
            </protocol>"#;
        let p = parse_protocol(src).unwrap();
        assert_eq!(p.name, "foo");
        assert_eq!(p.version, 1);
        assert_eq!(p.requests.len(), 1);
        assert_eq!(p.requests[0].name, "hello");
        assert_eq!(p.requests[0].opcode, 1);
        assert_eq!(p.requests[0].args[0].name, "x");
        assert_eq!(p.requests[0].args[0].ty, ArgType::U32);
    }

    #[test]
    fn parse_all_types() {
        let src = r#"<protocol name="t" version="1">
            <event name="m" opcode="1">
                <arg name="a" type="u32"/>
                <arg name="b" type="i32"/>
                <arg name="c" type="u64"/>
                <arg name="d" type="i64"/>
                <arg name="e" type="f32"/>
                <arg name="f" type="f64"/>
                <arg name="g" type="string"/>
                <arg name="h" type="rect"/>
                <arg name="i" type="kv_list"/>
                <arg name="j" type="array" element="u32"/>
                <arg name="k" type="array" element="string"/>
            </event>
        </protocol>"#;
        let p = parse_protocol(src).unwrap();
        let m = &p.events[0];
        assert_eq!(m.args.len(), 11);
        assert_eq!(m.args[9].ty, ArgType::Array(Box::new(ArgType::U32)));
        assert_eq!(m.args[10].ty, ArgType::Array(Box::new(ArgType::String)));
    }

    #[test]
    fn parse_fds() {
        let src = r#"<protocol name="t" version="1">
            <struct name="pool">
                <field name="count" type="u32"/>
                <field name="planes_per_buffer" type="u32"/>
            </struct>
            <event name="a" opcode="1"><fds count="1"/></event>
            <event name="b" opcode="2"><fds count="0"/></event>
            <event name="c" opcode="3">
                <arg name="pool" type="pool"/>
                <fds count_expr="pool.count * pool.planes_per_buffer"/>
            </event>
        </protocol>"#;
        let p = parse_protocol(src).unwrap();
        assert!(matches!(p.events[0].fds, FdSpec::Fixed(1)));
        assert!(matches!(p.events[1].fds, FdSpec::None));
        match &p.events[2].fds {
            FdSpec::Product(parts) => {
                assert_eq!(
                    parts,
                    &vec![
                        "pool.count".to_string(),
                        "pool.planes_per_buffer".to_string()
                    ]
                );
            }
            _ => panic!("expected Product"),
        }
    }

    #[test]
    fn rejects_invalid_fd_field_paths() {
        let missing = r#"<protocol name="t" version="1">
            <struct name="pool"><field name="count" type="u32"/></struct>
            <event name="bind" opcode="1">
                <arg name="pool" type="pool"/>
                <fds count_expr="pool.missing"/>
            </event>
        </protocol>"#;
        assert!(parse_protocol(missing).is_err());

        let non_integer = r#"<protocol name="t" version="1">
            <struct name="pool"><field name="label" type="string"/></struct>
            <event name="bind" opcode="1">
                <arg name="pool" type="pool"/>
                <fds count_expr="pool.label"/>
            </event>
        </protocol>"#;
        assert!(parse_protocol(non_integer).is_err());
    }

    #[test]
    fn comments_and_whitespace() {
        let src = r#"
            <!-- header comment -->
            <?xml version="1.0"?>
            <!-- another -->
            <protocol name="t" version="1">
                <!-- inside -->
                <request name="r" opcode="1"/>
            </protocol>"#;
        let p = parse_protocol(src).unwrap();
        assert_eq!(p.requests.len(), 1);
    }

    #[test]
    fn parses_and_orders_named_structs() {
        let src = r#"<protocol name="t" version="1">
            <struct name="outer">
                <field name="inner" type="inner"/>
                <field name="enabled" type="bool"/>
            </struct>
            <struct name="inner">
                <field name="generation" type="u64"/>
            </struct>
            <event name="configured" opcode="1">
                <arg name="config" type="outer"/>
            </event>
        </protocol>"#;
        let p = parse_protocol(src).unwrap();
        assert_eq!(p.structs.len(), 2);
        assert_eq!(p.structs[0].name, "inner");
        assert_eq!(p.structs[1].name, "outer");
        assert_eq!(p.events[0].args[0].ty, ArgType::Named("outer".to_string()));
    }

    #[test]
    fn parses_named_enums_in_structs_and_messages() {
        let src = r#"<protocol name="t" version="1">
            <enum name="effect_kind">
                <entry name="none" value="0"/>
                <entry name="blur" value="1"/>
            </enum>
            <struct name="config">
                <field name="kind" type="effect_kind"/>
            </struct>
            <event name="configured" opcode="1">
                <arg name="kind" type="effect_kind"/>
                <arg name="config" type="config"/>
            </event>
        </protocol>"#;
        let p = parse_protocol(src).unwrap();
        assert_eq!(p.enums.len(), 1);
        assert_eq!(p.enums[0].name, "effect_kind");
        assert_eq!(p.enums[0].entries[1].name, "blur");
        assert_eq!(p.enums[0].entries[1].value, 1);
        assert_eq!(
            p.structs[0].fields[0].ty,
            ArgType::Enum("effect_kind".to_string())
        );
        assert_eq!(
            p.events[0].args[0].ty,
            ArgType::Enum("effect_kind".to_string())
        );
    }

    #[test]
    fn rejects_invalid_named_enums() {
        let duplicate_value = r#"<protocol name="t" version="1">
            <enum name="kind">
                <entry name="one" value="1"/>
                <entry name="other" value="1"/>
            </enum>
        </protocol>"#;
        assert!(parse_protocol(duplicate_value)
            .unwrap_err()
            .msg
            .contains("duplicate value"));

        let duplicate_type = r#"<protocol name="t" version="1">
            <enum name="config"><entry name="one" value="1"/></enum>
            <struct name="config"><field name="value" type="u32"/></struct>
        </protocol>"#;
        assert!(parse_protocol(duplicate_type)
            .unwrap_err()
            .msg
            .contains("both an enum and a struct"));
    }

    #[test]
    fn rejects_unknown_and_recursive_structs() {
        let unknown = r#"<protocol name="t" version="1">
            <struct name="a"><field name="value" type="missing"/></struct>
        </protocol>"#;
        assert!(parse_protocol(unknown)
            .unwrap_err()
            .msg
            .contains("unknown type"));

        let recursive = r#"<protocol name="t" version="1">
            <struct name="a"><field name="value" type="b"/></struct>
            <struct name="b"><field name="value" type="a"/></struct>
        </protocol>"#;
        assert!(parse_protocol(recursive)
            .unwrap_err()
            .msg
            .contains("recursive struct reference"));
    }
}
