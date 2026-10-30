//! CSS stylesheet and declaration parsing.

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser,
};

use crate::compat::CompatReport;
use crate::snapshot::Viewport;

use super::selector::{Selector, parse_selector_list};

/// The origin of a declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Origin {
    /// Browser defaults.
    UserAgent,
    /// Author stylesheets.
    Author,
    /// The `style` attribute.
    Inline,
}

/// A declaration before it is applied to a computed style.
#[derive(Clone, Debug)]
pub(crate) struct Declaration {
    /// Property name in lowercase.
    pub(crate) name: String,
    /// Raw property value.
    pub(crate) value: String,
    /// Whether the declaration was marked `!important`.
    pub(crate) important: bool,
}

/// A media query attached to a rule.
#[derive(Clone, Debug, Default)]
pub(crate) struct MediaCondition {
    media_type: Option<String>,
    min_width: Option<f32>,
    max_width: Option<f32>,
    unsupported: bool,
}

impl MediaCondition {
    /// Returns true when the condition matches the viewport.
    pub(crate) fn matches(&self, viewport: Viewport) -> bool {
        if self.unsupported {
            return false;
        }
        if let Some(media_type) = &self.media_type {
            if media_type == "print" {
                return false;
            }
            if !matches!(media_type.as_str(), "all" | "screen" | "") {
                return false;
            }
        }
        let width = viewport.css_width();
        if let Some(min) = self.min_width {
            if width < min {
                return false;
            }
        }
        if let Some(max) = self.max_width {
            if width > max {
                return false;
            }
        }
        true
    }
}

/// A style rule.
#[derive(Clone, Debug)]
pub(crate) struct Rule {
    /// Selectors that trigger the rule.
    pub(crate) selectors: Vec<Selector>,
    /// Declarations in source order.
    pub(crate) declarations: Vec<Declaration>,
    /// Media conditions that must all match.
    pub(crate) media: Vec<MediaCondition>,
    /// Cascade origin.
    pub(crate) origin: Origin,
    /// Source order.
    pub(crate) order: u32,
}

impl Rule {
    /// Returns true when the rule applies to the viewport.
    pub(crate) fn applies(&self, viewport: Viewport) -> bool {
        self.media
            .iter()
            .all(|condition| condition.matches(viewport))
    }
}

/// Parses a stylesheet into rules.
pub(crate) fn parse_stylesheet(css: &str, origin: Origin, compat: &mut CompatReport) -> Vec<Rule> {
    let mut parser = Parser::new(css);
    let mut order = 0;
    parse_rules_from_parser(&mut parser, Vec::new(), origin, compat, &mut order)
}

fn parse_rules_from_parser(
    input: &mut Parser<'_>,
    media: Vec<MediaCondition>,
    origin: Origin,
    compat: &mut CompatReport,
    order: &mut u32,
) -> Vec<Rule> {
    let mut rule_parser = RuleParser {
        rules: Vec::new(),
        media,
        origin,
        compat,
        order,
    };
    let mut invalid = 0;
    for result in StyleSheetParser::new(input, &mut rule_parser) {
        if result.is_err() {
            invalid += 1;
        }
    }
    rule_parser.compat.ignored_rules += invalid;
    rule_parser.rules
}

struct RuleParser<'a> {
    rules: Vec<Rule>,
    media: Vec<MediaCondition>,
    origin: Origin,
    compat: &'a mut CompatReport,
    order: &'a mut u32,
}

impl<'i> QualifiedRuleParser<'i> for RuleParser<'_> {
    type Prelude = String;
    type QualifiedRule = ();
    type Error = ();

    fn parse_prelude(
        &mut self,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        let start = input.position();
        while input.next_including_whitespace_and_comments().is_ok() {}
        Ok(input.slice_from(start).to_owned())
    }

    fn parse_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Self::QualifiedRule, ParseError<Self::Error>> {
        let selectors = parse_selector_list(&prelude);
        if selectors.is_empty() {
            self.compat.ignored_rules += 1;
            while input.next().is_ok() {}
            return Ok(());
        }
        let declarations = parse_declarations_from_parser(input, self.compat);
        let order = *self.order;
        *self.order += 1;
        self.rules.push(Rule {
            selectors,
            declarations,
            media: self.media.clone(),
            origin: self.origin,
            order,
        });
        Ok(())
    }
}

impl<'i> AtRuleParser<'i> for RuleParser<'_> {
    type Prelude = AtRulePrelude;
    type AtRule = ();
    type Error = ();

    fn parse_prelude(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<Self::Prelude, ParseError<Self::Error>> {
        let start = input.position();
        while input.next_including_whitespace_and_comments().is_ok() {}
        Ok(AtRulePrelude {
            name: name.to_ascii_lowercase(),
            value: input.slice_from(start).trim().to_owned(),
        })
    }

    fn rule_without_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
    ) -> Result<Self::AtRule, ()> {
        self.compat
            .unsupported(format!("at-rule: @{}", prelude.name));
        Ok(())
    }

    fn parse_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Self::AtRule, ParseError<Self::Error>> {
        match prelude.name.as_str() {
            "media" => {
                let Some(condition) = parse_media(&prelude.value, self.compat) else {
                    while input.next().is_ok() {}
                    return Ok(());
                };
                let mut nested_media = self.media.clone();
                nested_media.push(condition);
                let nested = parse_rules_from_parser(
                    input,
                    nested_media,
                    self.origin,
                    self.compat,
                    self.order,
                );
                self.rules.extend(nested);
            }
            "font-face" => {
                self.compat.unsupported("at-rule: @font-face");
                while input.next().is_ok() {}
            }
            "supports" => {
                self.compat.unsupported("at-rule: @supports");
                while input.next().is_ok() {}
            }
            name => {
                self.compat.unsupported(format!("at-rule: @{name}"));
                while input.next().is_ok() {}
            }
        }
        Ok(())
    }
}

struct AtRulePrelude {
    name: String,
    value: String,
}

fn parse_media(raw: &str, compat: &mut CompatReport) -> Option<MediaCondition> {
    let raw = raw.trim().to_ascii_lowercase();
    if raw.is_empty() || raw == "all" {
        return Some(MediaCondition::default());
    }
    if raw == "screen" {
        return Some(MediaCondition {
            media_type: Some("screen".to_owned()),
            ..MediaCondition::default()
        });
    }
    if raw == "print" {
        return Some(MediaCondition {
            media_type: Some("print".to_owned()),
            ..MediaCondition::default()
        });
    }

    let mut condition = MediaCondition::default();
    let mut remaining = raw.as_str();
    if let Some(index) = remaining.find(" and ") {
        let media_type = remaining[..index].trim();
        condition.media_type = Some(media_type.to_owned());
        remaining = remaining[index + 5..].trim();
    } else if !remaining.starts_with('(') {
        let media_type = remaining.split_whitespace().next()?.to_owned();
        condition.media_type = Some(media_type);
        remaining = remaining[condition.media_type.as_ref()?.len()..].trim();
    }

    while let Some(start) = remaining.find('(') {
        let Some(end) = remaining[start..].find(')') else {
            compat.unsupported("media query: unterminated feature");
            condition.unsupported = true;
            break;
        };
        let feature = remaining[start + 1..start + end].trim();
        if let Some(value) = feature.strip_prefix("min-width:") {
            if let Some(width) = parse_media_length(value.trim()) {
                condition.min_width = Some(width);
            } else {
                condition.unsupported = true;
            }
        } else if let Some(value) = feature.strip_prefix("max-width:") {
            if let Some(width) = parse_media_length(value.trim()) {
                condition.max_width = Some(width);
            } else {
                condition.unsupported = true;
            }
        } else if feature == "screen" || feature == "color" || feature == "monochrome" {
            // Known but not width-dependent.
        } else {
            compat.unsupported(format!("media feature: {feature}"));
            condition.unsupported = true;
        }
        remaining = &remaining[start + end + 1..];
    }

    Some(condition)
}

fn parse_media_length(raw: &str) -> Option<f32> {
    if let Some(value) = raw.strip_suffix("px") {
        value.parse().ok()
    } else if let Some(value) = raw.strip_suffix("em") {
        value.parse::<f32>().ok().map(|value| value * 16.0)
    } else if let Some(value) = raw.strip_suffix("rem") {
        value.parse::<f32>().ok().map(|value| value * 16.0)
    } else {
        raw.parse().ok()
    }
}

fn parse_declarations_from_parser(
    input: &mut Parser<'_>,
    compat: &mut CompatReport,
) -> Vec<Declaration> {
    let mut collector = DeclarationCollector {
        declarations: Vec::new(),
        compat,
    };
    for result in RuleBodyParser::new(input, &mut collector) {
        if result.is_err() {
            // Invalid declarations are skipped by the CSS syntax rules.
        }
    }
    collector.declarations
}

/// Parses a declaration list, such as the content of a `style` attribute.
pub(crate) fn parse_declarations(raw: &str, compat: &mut CompatReport) -> Vec<Declaration> {
    let mut parser = Parser::new(raw);
    parse_declarations_from_parser(&mut parser, compat)
}

struct DeclarationCollector<'a> {
    declarations: Vec<Declaration>,
    compat: &'a mut CompatReport,
}

impl<'i> DeclarationParser<'i> for DeclarationCollector<'_> {
    type Declaration = ();
    type Error = ();

    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _declaration_start: &ParserState,
    ) -> Result<Self::Declaration, ParseError<Self::Error>> {
        let start = input.position();
        while input.next_including_whitespace_and_comments().is_ok() {}
        let raw = input.slice_from(start).trim();
        let (value, important) = split_important(raw);
        let name = name.to_ascii_lowercase();
        if !is_known_property(&name) && !name.starts_with("--") {
            self.compat.unsupported(format!("property: {name}"));
        }
        self.declarations.push(Declaration {
            name,
            value,
            important,
        });
        Ok(())
    }
}

impl<'i> QualifiedRuleParser<'i> for DeclarationCollector<'_> {
    type Prelude = ();
    type QualifiedRule = ();
    type Error = ();
}

impl<'i> AtRuleParser<'i> for DeclarationCollector<'_> {
    type Prelude = ();
    type AtRule = ();
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, (), ()> for DeclarationCollector<'_> {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

fn split_important(raw: &str) -> (String, bool) {
    let trimmed = raw.trim_end();
    if let Some(index) = trimmed.rfind('!') {
        let tail = trimmed[index + 1..].trim();
        if tail.eq_ignore_ascii_case("important") {
            return (trimmed[..index].trim().to_owned(), true);
        }
    }
    (trimmed.to_owned(), false)
}

fn is_known_property(name: &str) -> bool {
    matches!(
        name,
        "display"
            | "position"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "width"
            | "height"
            | "min-width"
            | "min-height"
            | "max-width"
            | "max-height"
            | "box-sizing"
            | "margin"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "padding"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "border"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-width"
            | "border-style"
            | "border-color"
            | "border-radius"
            | "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width"
            | "border-top-style"
            | "border-right-style"
            | "border-bottom-style"
            | "border-left-style"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "color"
            | "background"
            | "background-color"
            | "font"
            | "font-family"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "line-height"
            | "text-align"
            | "text-transform"
            | "letter-spacing"
            | "text-decoration"
            | "white-space"
            | "overflow"
            | "overflow-x"
            | "overflow-y"
            | "opacity"
            | "visibility"
            | "flex"
            | "flex-direction"
            | "flex-wrap"
            | "flex-grow"
            | "flex-shrink"
            | "flex-basis"
            | "justify-content"
            | "align-items"
            | "align-self"
            | "grid-template-columns"
            | "gap"
            | "row-gap"
            | "column-gap"
            | "list-style"
            | "list-style-type"
            | "vertical-align"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rules_and_declarations() {
        let mut compat = CompatReport::default();
        let rules = parse_stylesheet(
            "h1 { color: red; margin: 0 !important; } @media (max-width: 500px) { p { color: blue } }",
            Origin::Author,
            &mut compat,
        );
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].declarations.len(), 2);
        assert_eq!(rules[0].declarations[0].name, "color");
        assert_eq!(rules[0].declarations[0].value, "red");
        assert!(rules[0].declarations[1].important);
        assert_eq!(rules[1].declarations[0].value, "blue");
        assert!(compat.is_empty(), "{:?}", compat);
    }

    #[test]
    fn reports_unknown_properties_without_dropping_known_ones() {
        let mut compat = CompatReport::default();
        let declarations =
            parse_declarations("color: red; filter: blur(1px); --accent: blue", &mut compat);
        assert_eq!(declarations.len(), 3);
        assert!(
            compat
                .unsupported
                .iter()
                .any(|feature| feature == "property: filter")
        );
        assert!(
            !compat
                .unsupported
                .iter()
                .any(|feature| feature == "property: --accent")
        );
    }
}
