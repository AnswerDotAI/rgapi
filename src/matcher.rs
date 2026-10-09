use fancy_regex::{Assertion, Expr, Regex, RegexBuilder};
use grep_matcher::{LineTerminator, Match, Matcher, NoCaptures};
use regex_syntax::{ast::{self, Ast, Visitor}, hir::{Class, Hir, HirKind}};

use crate::RgApiError;

/// A fancy-regex matcher for grep-searcher. Ordinary line-safe patterns search whole
/// buffers; other line-oriented patterns search individual lines, without terminators.
/// Both paths use the same regex engine and return UTF-8 byte offsets.
#[derive(Debug, Clone)]
pub struct RegexMatcher { regex: Regex, line_oriented: bool, whole_buffer: bool }

impl RegexMatcher {
    pub(crate) fn path(pattern: &str, smart_case: bool) -> Result<Self, RgApiError> { Self::build(pattern, None, smart_case, false, false) }

    fn build(pattern: &str, case_sensitive: Option<bool>, smart_case: bool, line_oriented: bool, line_anchors: bool) -> Result<Self, RgApiError> {
        // Parse with the same anchor defaults as the compiled regex. Fancy's resolved
        // tree excludes group names, escape names and comments from smart-case analysis.
        let parsed = if line_anchors { format!("(?mR){pattern}") } else { pattern.to_string() };
        let tree = Expr::parse_tree(&parsed).map_err(regex_error)?;
        let mut analysis = Analysis::default();
        let mut stack = vec![&tree.expr];
        while let Some(expr) = stack.pop() {
            analysis.expression(expr)?;
            stack.extend(expr.children_iter());
        }
        if line_oriented && analysis.literal_newline { return Err(RgApiError::new("a literal newline or carriage return is not allowed in a regex without multiline=True")); }
        let insensitive = case_sensitive.map_or(smart_case && analysis.literal && !analysis.uppercase, |s| !s);
        let regex = RegexBuilder::new(pattern).multi_line(line_anchors).crlf(line_anchors).case_insensitive(insensitive).build().map_err(regex_error)?;
        Ok(Self { regex, line_oriented, whole_buffer: !line_oriented || !analysis.line_local })
    }

    pub(crate) fn for_each_match(&self, text: &str, mut matched: impl FnMut(usize, usize)) -> Result<(), fancy_regex::Error> {
        let mut scan = |part: &str, offset| {
            for m in self.regex.find_iter(part) { let m = m?; matched(offset + m.start(), offset + m.end()); }
            Ok(())
        };
        if self.whole_buffer { return scan(text, 0); }
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            let body = line.strip_suffix('\n').unwrap_or(line);
            scan(body.strip_suffix('\r').unwrap_or(body), offset)?;
            offset += line.len();
        }
        if text.is_empty() { scan(text, 0)?; }
        Ok(())
    }
}

/// Compile a content regex, including backreferences and lookaround.
/// With `multiline=false`, matching is confined to individual lines and literal
/// line endings are rejected. With `multiline=true`, the whole input is searched.
pub fn compile_regex(pattern: &str, case_sensitive: Option<bool>, smart_case: bool, multiline: bool) -> Result<RegexMatcher, RgApiError> {
    if pattern.is_empty() { return Err(RgApiError::new("pattern may not be empty")); }
    RegexMatcher::build(pattern, case_sensitive, smart_case, !multiline, true)
}

fn regex_error(error: impl std::fmt::Display) -> RgApiError { RgApiError::new(error.to_string()) }

impl Matcher for RegexMatcher {
    type Captures = NoCaptures;
    type Error = fancy_regex::Error;

    fn new_captures(&self) -> Result<NoCaptures, Self::Error> { Ok(NoCaptures::new()) }
    fn line_terminator(&self) -> Option<LineTerminator> { self.line_oriented.then(LineTerminator::crlf) }

    fn find_at(&self, haystack: &[u8], mut at: usize) -> Result<Option<Match>, Self::Error> {
        if at > haystack.len() { return Ok(None); }
        // grep-matcher advances empty matches by one byte; fancy's VM must never
        // start its next search in the middle of a UTF-8 character.
        while at < haystack.len() && haystack[at] & 0xC0 == 0x80 { at += 1; }
        if self.whole_buffer { return self.regex.find_from_pos(haystack, at).map(|m| m.map(|m| Match::new(m.start(), m.end()))); }
        // Keep the beginning of the current line for anchors and lookbehind when
        // grep-matcher asks for another match part-way through it.
        let mut start = haystack[..at].iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        loop {
            if start > 0 && start == haystack.len() { return Ok(None); }
            let end = haystack[start..].iter().position(|&b| b == b'\n').map_or(haystack.len(), |i| start + i);
            let line = &haystack[start..end];
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let pos = at.saturating_sub(start);
            if pos <= line.len() && let Some(m) = self.regex.find_from_pos(line, pos)? { return Ok(Some(Match::new(start + m.start(), start + m.end()))); }
            if end == haystack.len() { return Ok(None); }
            start = end + 1;
        }
    }
}

#[derive(Default)]
struct Analysis { literal: bool, uppercase: bool, literal_newline: bool, line_local: bool }

impl Analysis {
    fn literal(&mut self, c: char) { self.literal = true; self.uppercase |= c.is_uppercase(); }

    fn expression(&mut self, expr: &Expr) -> Result<(), RgApiError> {
        match expr {
            Expr::Literal { val, .. } => {
                for c in val.chars() { self.literal(c); }
                self.literal_newline |= val.contains(['\r', '\n']);
            }
            Expr::Delegate { inner, .. } => {
                let ast = ast::parse::Parser::new().parse(inner).map_err(regex_error)?;
                let literals = ast::visit(&ast, ClassLiterals::default()).unwrap();
                self.literal |= literals.literal;
                self.uppercase |= literals.uppercase;
                let hir = regex_syntax::Parser::new().parse(inner).map_err(regex_error)?;
                self.line_local |= contains_newline(&hir);
                if let HirKind::Literal(lit) = hir.kind() { self.literal_newline |= lit.0.iter().any(|b| matches!(b, b'\r' | b'\n')); }
            }
            Expr::Any { newline, crlf } => self.line_local |= *newline || !crlf,
            Expr::Assertion(Assertion::StartText | Assertion::EndText | Assertion::EndTextIgnoreTrailingNewlines { .. }) => self.line_local = true,
            Expr::Empty | Expr::Assertion(_) | Expr::Concat(_) | Expr::Alt(_) | Expr::Group(_) | Expr::Repeat { .. } => {}
            // Backreferences, lookaround and other fancy constructs use line-local
            // input so captures and assertions cannot inspect neighbouring lines.
            _ => self.line_local = true,
        }
        Ok(())
    }
}

fn contains_newline(hir: &Hir) -> bool {
    match hir.kind() {
        HirKind::Literal(lit) => lit.0.iter().any(|b| matches!(b, b'\r' | b'\n')),
        HirKind::Class(Class::Unicode(c)) => c.ranges().iter().any(|r| ['\r', '\n'].iter().any(|&c| r.start() <= c && c <= r.end())),
        HirKind::Class(Class::Bytes(c)) => c.ranges().iter().any(|r| [b'\r', b'\n'].iter().any(|&c| r.start() <= c && c <= r.end())),
        _ => hir.kind().subs().iter().any(contains_newline),
    }
}

#[derive(Default)]
struct ClassLiterals { literal: bool, uppercase: bool }
impl ClassLiterals { fn add(&mut self, c: char) { self.literal = true; self.uppercase |= c.is_uppercase(); } }
impl Visitor for ClassLiterals {
    type Output = Self;
    type Err = std::convert::Infallible;
    fn finish(self) -> Result<Self, Self::Err> { Ok(self) }
    fn visit_pre(&mut self, ast: &Ast) -> Result<(), Self::Err> { if let Ast::Literal(lit) = ast { self.add(lit.c); } Ok(()) }
    fn visit_class_set_item_pre(&mut self, item: &ast::ClassSetItem) -> Result<(), Self::Err> {
        match item { ast::ClassSetItem::Literal(lit) => self.add(lit.c), ast::ClassSetItem::Range(r) => { self.add(r.start.c); self.add(r.end.c); } _ => {} }
        Ok(())
    }
}
