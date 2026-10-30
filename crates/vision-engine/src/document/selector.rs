//! CSS selector parsing and matching.
//!
//! The engine implements the selector subset that participates in layout and
//! audit checks. Unsupported selectors are discarded by the stylesheet parser
//! and reported through [`crate::compat::CompatReport`].

use crate::document::dom::{Document, NodeId};

/// A parsed selector with its specificity.
#[derive(Clone, Debug)]
pub(crate) struct Selector {
    parts: Vec<SelectorPart>,
    specificity: u32,
}

#[derive(Clone, Debug)]
struct SelectorPart {
    compound: CompoundSelector,
    combinator: Option<Combinator>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Combinator {
    Descendant,
    Child,
    Adjacent,
    General,
}

#[derive(Clone, Debug, Default)]
struct CompoundSelector {
    universal: bool,
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<AttributeSelector>,
    pseudos: Vec<PseudoClass>,
}

#[derive(Clone, Debug)]
struct AttributeSelector {
    name: String,
    operator: Option<AttributeOperator>,
    value: String,
    case_insensitive: bool,
}

#[derive(Clone, Copy, Debug)]
enum AttributeOperator {
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Clone, Debug)]
enum PseudoClass {
    Root,
    FirstChild,
    LastChild,
    OnlyChild,
    Empty,
    Checked,
    Disabled,
    Enabled,
    Link,
    Hover,
    Focus,
    Active,
    Visited,
    Not(Box<CompoundSelector>),
    NthChild(Nth),
}

#[derive(Clone, Copy, Debug)]
enum Nth {
    Odd,
    Even,
    Number(i32),
    AnPlusB(i32, i32),
}

impl Selector {
    /// Returns the selector specificity.
    pub(crate) fn specificity(&self) -> u32 {
        self.specificity
    }

    /// Returns true when the selector matches the node.
    pub(crate) fn matches(&self, document: &Document, node: NodeId) -> bool {
        if self.parts.is_empty() {
            return false;
        }
        let mut current = node;
        for index in (0..self.parts.len()).rev() {
            if !self.parts[index].compound.matches(document, current) {
                return false;
            }
            if index == 0 {
                return true;
            }
            let combinator = self.parts[index]
                .combinator
                .unwrap_or(Combinator::Descendant);
            let previous = &self.parts[index - 1].compound;
            current = match combinator {
                Combinator::Child => match document.parent(current) {
                    Some(parent) if previous.matches(document, parent) => parent,
                    _ => return false,
                },
                Combinator::Descendant => {
                    let mut ancestor = document.parent(current);
                    loop {
                        match ancestor {
                            Some(candidate) if previous.matches(document, candidate) => {
                                break candidate;
                            }
                            Some(candidate) => ancestor = document.parent(candidate),
                            None => return false,
                        }
                    }
                }
                Combinator::Adjacent => match document.node(current).prev_sibling {
                    Some(sibling) if previous.matches(document, sibling) => sibling,
                    _ => return false,
                },
                Combinator::General => {
                    let mut sibling = document.node(current).prev_sibling;
                    loop {
                        match sibling {
                            Some(candidate) if previous.matches(document, candidate) => {
                                break candidate;
                            }
                            Some(candidate) => sibling = document.node(candidate).prev_sibling,
                            None => return false,
                        }
                    }
                }
            };
        }
        true
    }
}

impl CompoundSelector {
    fn matches(&self, document: &Document, node: NodeId) -> bool {
        let Some(element) = document.element(node) else {
            return false;
        };
        if let Some(tag) = &self.tag {
            if !element.name.eq_ignore_ascii_case(tag) {
                return false;
            }
        }
        if let Some(id) = &self.id {
            if document.attr(node, "id") != Some(id.as_str()) {
                return false;
            }
        }
        if !self.classes.is_empty() {
            let classes = document.classes(node);
            if !self
                .classes
                .iter()
                .all(|class| classes.contains(&class.as_str()))
            {
                return false;
            }
        }
        for attribute in &self.attrs {
            if !attribute.matches(document, node) {
                return false;
            }
        }
        for pseudo in &self.pseudos {
            if !pseudo.matches(document, node) {
                return false;
            }
        }
        true
    }

    fn specificity(&self) -> u32 {
        let mut specificity = 0;
        if self.tag.is_some() {
            specificity += 1;
        }
        if self.id.is_some() {
            specificity += 100;
        }
        specificity += (self.classes.len() + self.attrs.len()) as u32 * 10;
        for pseudo in &self.pseudos {
            specificity += match pseudo {
                PseudoClass::Not(inner) => inner.specificity(),
                _ => 10,
            };
        }
        specificity
    }

    fn is_empty(&self) -> bool {
        !self.universal
            && self.tag.is_none()
            && self.id.is_none()
            && self.classes.is_empty()
            && self.attrs.is_empty()
            && self.pseudos.is_empty()
    }
}

impl AttributeSelector {
    fn matches(&self, document: &Document, node: NodeId) -> bool {
        let Some(actual) = document.attr(node, &self.name) else {
            return false;
        };
        let Some(operator) = self.operator else {
            return true;
        };
        let (actual, expected) = if self.case_insensitive {
            (actual.to_ascii_lowercase(), self.value.to_ascii_lowercase())
        } else {
            (actual.to_owned(), self.value.clone())
        };
        match operator {
            AttributeOperator::Equals => actual == expected,
            AttributeOperator::Includes => actual
                .split_ascii_whitespace()
                .any(|value| value == expected),
            AttributeOperator::DashMatch => {
                actual == expected || actual.starts_with(&format!("{expected}-"))
            }
            AttributeOperator::Prefix => actual.starts_with(&expected),
            AttributeOperator::Suffix => actual.ends_with(&expected),
            AttributeOperator::Substring => actual.contains(&expected),
        }
    }
}

impl PseudoClass {
    fn matches(&self, document: &Document, node: NodeId) -> bool {
        match self {
            Self::Root => document.element_name(node) == Some("html"),
            Self::FirstChild => document.node(node).prev_sibling.is_none(),
            Self::LastChild => document.node(node).next_sibling.is_none(),
            Self::OnlyChild => {
                document.node(node).prev_sibling.is_none()
                    && document.node(node).next_sibling.is_none()
            }
            Self::Empty => document.children(node).next().is_none(),
            Self::Checked => {
                document.attr(node, "checked").is_some()
                    || document.attr(node, "selected").is_some()
            }
            Self::Disabled => document.attr(node, "disabled").is_some(),
            Self::Enabled => {
                document.attr(node, "disabled").is_none()
                    && matches!(
                        document.element_name(node),
                        Some("button" | "input" | "select" | "textarea" | "option")
                    )
            }
            Self::Link => {
                document.element_name(node) == Some("a") && document.attr(node, "href").is_some()
            }
            Self::Hover | Self::Focus | Self::Active | Self::Visited => false,
            Self::Not(inner) => !inner.matches(document, node),
            Self::NthChild(nth) => {
                let mut index = 1;
                let mut sibling = document.node(node).prev_sibling;
                while let Some(candidate) = sibling {
                    index += 1;
                    sibling = document.node(candidate).prev_sibling;
                }
                nth.matches(index)
            }
        }
    }
}

impl Nth {
    fn matches(self, index: i32) -> bool {
        match self {
            Self::Odd => index % 2 == 1,
            Self::Even => index % 2 == 0,
            Self::Number(number) => index == number,
            Self::AnPlusB(a, b) => {
                if a == 0 {
                    index == b
                } else {
                    let delta = index - b;
                    delta % a == 0 && delta / a >= 0
                }
            }
        }
    }
}

/// Parses a comma-separated selector list.
pub(crate) fn parse_selector_list(input: &str) -> Vec<Selector> {
    let input = strip_comments(input);
    split_top_level(&input, ',')
        .into_iter()
        .filter_map(|part| parse_complex(part.trim()))
        .collect()
}

fn parse_complex(input: &str) -> Option<Selector> {
    let mut parser = SelectorParser::new(input);
    let first = parser.parse_compound()?;
    let mut parts = vec![SelectorPart {
        compound: first,
        combinator: None,
    }];
    loop {
        let had_space = parser.skip_space();
        if parser.is_end() {
            break;
        }
        let combinator = match parser.peek() {
            Some('>') => {
                parser.advance();
                Combinator::Child
            }
            Some('+') => {
                parser.advance();
                Combinator::Adjacent
            }
            Some('~') => {
                parser.advance();
                Combinator::General
            }
            _ if had_space => Combinator::Descendant,
            _ => return None,
        };
        parser.skip_space();
        let compound = parser.parse_compound()?;
        parts.push(SelectorPart {
            compound,
            combinator: Some(combinator),
        });
    }
    let specificity = parts
        .iter()
        .map(|part| part.compound.specificity())
        .sum::<u32>();
    Some(Selector { parts, specificity })
}

struct SelectorParser {
    chars: Vec<char>,
    pos: usize,
}

impl SelectorParser {
    fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn is_end(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let value = self.peek()?;
        self.pos += 1;
        Some(value)
    }

    fn skip_space(&mut self) -> bool {
        let start = self.pos;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_whitespace()) {
            self.pos += 1;
        }
        self.pos > start
    }

    fn parse_compound(&mut self) -> Option<CompoundSelector> {
        let mut compound = CompoundSelector::default();
        if self.peek() == Some('*') {
            self.advance();
            compound.universal = true;
        } else if matches!(self.peek(), Some(ch) if is_ident_start(ch)) {
            compound.tag = Some(self.parse_ident()?);
        }

        loop {
            match self.peek() {
                Some('.') => {
                    self.advance();
                    compound.classes.push(self.parse_ident()?);
                }
                Some('#') => {
                    self.advance();
                    compound.id = Some(self.parse_ident()?);
                }
                Some('[') => {
                    compound.attrs.push(self.parse_attribute()?);
                }
                Some(':') => {
                    self.advance();
                    if self.peek() == Some(':') {
                        // Pseudo-elements are not rendered yet.
                        return None;
                    }
                    compound.pseudos.push(self.parse_pseudo()?);
                }
                Some(ch)
                    if ch.is_ascii_whitespace()
                        || matches!(ch, '>' | '+' | '~' | ',' | ')' | ']') =>
                {
                    break;
                }
                Some(_) => return None,
                None => break,
            }
        }

        if compound.is_empty() {
            None
        } else {
            Some(compound)
        }
    }

    fn parse_ident(&mut self) -> Option<String> {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '\\') {
                self.pos += 1;
                if ch == '\\' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
        (self.pos > start).then(|| self.chars[start..self.pos].iter().collect())
    }

    fn parse_attribute(&mut self) -> Option<AttributeSelector> {
        self.advance();
        self.skip_space();
        let name = self.parse_ident()?.to_ascii_lowercase();
        self.skip_space();
        if self.peek() == Some(']') {
            self.advance();
            return Some(AttributeSelector {
                name,
                operator: None,
                value: String::new(),
                case_insensitive: false,
            });
        }
        let operator = match self.advance()? {
            '=' => AttributeOperator::Equals,
            '~' => {
                if self.advance()? != '=' {
                    return None;
                }
                AttributeOperator::Includes
            }
            '|' => {
                if self.advance()? != '=' {
                    return None;
                }
                AttributeOperator::DashMatch
            }
            '^' => {
                if self.advance()? != '=' {
                    return None;
                }
                AttributeOperator::Prefix
            }
            '$' => {
                if self.advance()? != '=' {
                    return None;
                }
                AttributeOperator::Suffix
            }
            '*' => {
                if self.advance()? != '=' {
                    return None;
                }
                AttributeOperator::Substring
            }
            _ => return None,
        };
        self.skip_space();
        let value = if matches!(self.peek(), Some('"' | '\'')) {
            self.parse_quoted()?
        } else {
            self.parse_ident()?
        };
        self.skip_space();
        let case_insensitive = if matches!(self.peek(), Some('i' | 'I')) {
            self.advance();
            true
        } else {
            false
        };
        self.skip_space();
        if self.advance()? != ']' {
            return None;
        }
        Some(AttributeSelector {
            name,
            operator: Some(operator),
            value,
            case_insensitive,
        })
    }

    fn parse_quoted(&mut self) -> Option<String> {
        let quote = self.advance()?;
        let mut value = String::new();
        while let Some(ch) = self.advance() {
            if ch == quote {
                return Some(value);
            }
            value.push(ch);
        }
        None
    }

    fn parse_pseudo(&mut self) -> Option<PseudoClass> {
        let name = self.parse_ident()?.to_ascii_lowercase();
        match name.as_str() {
            "root" => Some(PseudoClass::Root),
            "first-child" => Some(PseudoClass::FirstChild),
            "last-child" => Some(PseudoClass::LastChild),
            "only-child" => Some(PseudoClass::OnlyChild),
            "empty" => Some(PseudoClass::Empty),
            "checked" => Some(PseudoClass::Checked),
            "disabled" => Some(PseudoClass::Disabled),
            "enabled" => Some(PseudoClass::Enabled),
            "link" | "any-link" => Some(PseudoClass::Link),
            "hover" => Some(PseudoClass::Hover),
            "focus" | "focus-visible" | "focus-within" => Some(PseudoClass::Focus),
            "active" => Some(PseudoClass::Active),
            "visited" => Some(PseudoClass::Visited),
            "not" => {
                if self.advance()? != '(' {
                    return None;
                }
                self.skip_space();
                let inner = self.parse_compound()?;
                self.skip_space();
                if self.advance()? != ')' {
                    return None;
                }
                Some(PseudoClass::Not(Box::new(inner)))
            }
            "nth-child" => {
                if self.advance()? != '(' {
                    return None;
                }
                let start = self.pos;
                while let Some(ch) = self.peek() {
                    if ch == ')' {
                        break;
                    }
                    self.pos += 1;
                }
                let raw: String = self.chars[start..self.pos].iter().collect();
                if self.advance()? != ')' {
                    return None;
                }
                Some(PseudoClass::NthChild(parse_nth(raw.trim())?))
            }
            _ => None,
        }
    }
}

fn parse_nth(raw: &str) -> Option<Nth> {
    let raw = raw.replace(' ', "").to_ascii_lowercase();
    if raw == "odd" {
        return Some(Nth::Odd);
    }
    if raw == "even" {
        return Some(Nth::Even);
    }
    if let Ok(number) = raw.parse::<i32>() {
        return Some(Nth::Number(number));
    }
    let index = raw.find('n')?;
    let coefficient = &raw[..index];
    let rest = &raw[index + 1..];
    let a = match coefficient {
        "" | "+" => 1,
        "-" => -1,
        _ => coefficient.parse().ok()?,
    };
    let b = if rest.is_empty() {
        0
    } else {
        rest.parse().ok()?
    };
    Some(Nth::AnPlusB(a, b))
}

fn strip_comments(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(comment) = chars.next() {
                if comment == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
        } else {
            output.push(ch);
        }
    }
    output
}

fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ if ch == delimiter && depth == 0 => {
                parts.push(&input[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&input[start..]);
    parts
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || matches!(ch, '-' | '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::error::Result;

    fn element(document: &Document, predicate: impl Fn(&Document, NodeId) -> bool) -> NodeId {
        document
            .descendants()
            .find(|node| predicate(document, *node))
            .unwrap_or_else(|| panic!("missing test element"))
    }

    #[test]
    fn matches_compound_and_child() -> Result<()> {
        let document = Document::parse(
            "<div class=card data-kind=box><p id=title>Hi</p><p class=muted>Bye</p></div>",
        )?;
        let title = element(&document, |document, node| {
            document.attr(node, "id") == Some("title")
        });
        let muted = element(&document, |document, node| {
            document.attr(node, "class") == Some("muted")
        });

        let selector = parse_selector_list("div.card > p#title");
        let Some(selector) = selector.first() else {
            panic!("selector did not parse");
        };
        assert!(selector.matches(&document, title));
        assert!(!selector.matches(&document, muted));

        let selector = parse_selector_list("p:not(#title)");
        let Some(selector) = selector.first() else {
            panic!("selector did not parse");
        };
        assert!(!selector.matches(&document, title));
        assert!(selector.matches(&document, muted));

        let selector = parse_selector_list("div[data-kind=box]");
        let Some(selector) = selector.first() else {
            panic!("selector did not parse");
        };
        let div = document
            .parent(title)
            .unwrap_or_else(|| panic!("missing parent"));
        assert!(selector.matches(&document, div));
        Ok(())
    }

    #[test]
    fn matches_nth_child() -> Result<()> {
        let document = Document::parse("<ul><li>One</li><li>Two</li></ul>")?;
        let second = document
            .descendants()
            .filter(|node| document.element_name(*node) == Some("li"))
            .nth(1)
            .unwrap_or_else(|| panic!("missing second list item"));
        let selector = parse_selector_list("li:nth-child(2)");
        let Some(selector) = selector.first() else {
            panic!("selector did not parse");
        };
        assert!(selector.matches(&document, second));
        Ok(())
    }
}
