//! A bounded shell lexer, not an interpreter. All contexts share one source cursor.
use crate::ConfigKey;

pub(super) fn scan(source: &str) -> Vec<(usize, Option<ConfigKey>)> {
    let mut lexer = Lexer {
        source: source.as_bytes(),
        at: 0,
        line: 1,
        references: Vec::new(),
    };
    lexer.commands(None, 0, &mut Vec::new(), false);
    lexer.references.sort_by_key(|(line, _)| *line);
    lexer.references
}

struct Word {
    text: String,
    dynamic: bool,
    escaped: bool,
    plain: bool,
    assignment: bool,
    line: usize,
}
impl Word {
    fn name_length(source: &[u8]) -> usize {
        source
            .iter()
            .enumerate()
            .take_while(|(i, b)| {
                **b == b'_' || b.is_ascii_alphabetic() || (*i > 0 && b.is_ascii_digit())
            })
            .count()
    }
    // Assignment is a source role, not a property of the quote-decoded value.
    fn assignment(source: &[u8]) -> bool {
        let name = Self::name_length(source);
        if name == 0 {
            return false;
        }
        let mut rest = &source[name..];
        if rest.starts_with(b"[") {
            let Some(end) = rest.iter().position(|b| *b == b']') else {
                return false;
            };
            rest = &rest[end + 1..];
        }
        rest.starts_with(b"=") || rest.starts_with(b"+=")
    }
}
struct Lexer<'a> {
    source: &'a [u8],
    at: usize,
    line: usize,
    references: Vec<(usize, Option<ConfigKey>)>,
}
impl Lexer<'_> {
    fn starts(&self, text: &[u8]) -> bool {
        self.source[self.at..].starts_with(text)
    }
    fn advance(&mut self) {
        if self.source.get(self.at) == Some(&b'\n') {
            self.line += 1;
        }
        self.at += 1;
    }
    fn spaces(&mut self) {
        while self.at < self.source.len() {
            if self.starts(b"\\\n") {
                self.advance();
                self.advance();
            } else if matches!(self.source[self.at], b' ' | b'\t' | b'\r') {
                self.advance();
            } else {
                break;
            }
        }
    }
    fn unknown(&mut self, line: usize) {
        self.references.push((line, None));
    }

    // ponytail: bound nesting at 16; unsupported boundaries fail closed through EOF,
    // rather than reinterpreting uncertain data as a new shell command.
    fn commands(
        &mut self,
        end: Option<u8>,
        depth: usize,
        heredocs: &mut Vec<(String, bool)>,
        data: bool,
    ) -> bool {
        if depth >= 16 {
            self.unknown(self.line);
            self.at = self.source.len();
            return false;
        }
        let mut words = Vec::new();
        let mut redirected = false;
        while self.at < self.source.len() {
            self.spaces();
            let Some(&byte) = self.source.get(self.at) else {
                break;
            };
            if Some(byte) == end {
                self.invocation(&words, redirected, data);
                self.advance();
                return true;
            }
            match byte {
                b'#' => {
                    // We are between words/operators here; a # within a word is literal.
                    while self.at < self.source.len() && self.source[self.at] != b'\n' {
                        self.advance();
                    }
                }
                b'\n' | b';' | b'|' | b'&' => {
                    self.invocation(&words, redirected, data);
                    words.clear();
                    redirected = false;
                    self.advance();
                    if byte == b'\n' {
                        self.heredoc_bodies(heredocs);
                    }
                }
                b'(' => {
                    if !data && self.starts(b"((") {
                        self.at += 2;
                        self.arithmetic(depth);
                        continue;
                    }
                    let compound = data
                        || (self.source.get(self.at.wrapping_sub(1)) == Some(&b'=')
                            && words.last().is_some_and(|w: &Word| w.assignment));
                    if !compound {
                        self.invocation(&words, redirected, data);
                        words.clear();
                        redirected = false;
                    }
                    self.advance();
                    self.commands(Some(b')'), depth + 1, heredocs, compound);
                }
                b')' => {
                    self.invocation(&words, redirected, data);
                    words.clear();
                    self.advance();
                }
                b'<' | b'>' => {
                    // Adjacent unquoted numeric/named descriptors are operator roles, not argv.
                    if self.at > 0
                        && !self.source[self.at - 1].is_ascii_whitespace()
                        && words.last().is_some_and(|w| {
                            w.plain
                                && (w.text.bytes().all(|b| b.is_ascii_digit())
                                    || w.text
                                        .strip_prefix('{')
                                        .and_then(|s| s.strip_suffix('}'))
                                        .is_some_and(|name| {
                                            !name.is_empty()
                                                && Word::name_length(name.as_bytes()) == name.len()
                                        }))
                        })
                    {
                        words.pop();
                    }
                    redirected |= words.is_empty();
                    let line = self.line;
                    let heredoc = self.starts(b"<<") && !self.starts(b"<<<");
                    let here_string = self.starts(b"<<<");
                    self.advance();
                    if self.source.get(self.at) == Some(&byte) {
                        self.advance();
                    }
                    if here_string {
                        self.advance();
                    }
                    let tabs = heredoc && self.starts(b"-");
                    if tabs {
                        self.advance();
                    }
                    // Descriptor duplication is still a redirection operand, not a command.
                    if matches!(self.source.get(self.at), Some(b'&' | b'|')) {
                        self.advance();
                    }
                    self.spaces();
                    if heredoc || here_string {
                        self.unknown(line);
                    }
                    if let Some(word) = self.word(heredoc, end, depth) {
                        if heredoc {
                            heredocs.push((word.text, tabs));
                        }
                    } else if heredoc {
                        self.at = self.source.len();
                    }
                }
                _ => {
                    if let Some(word) = self.word(false, end, depth) {
                        words.push(word);
                    } else {
                        self.unknown(self.line);
                        self.at = self.source.len();
                    }
                }
            }
        }
        self.invocation(&words, redirected, data);
        end.is_none()
    }

    fn heredoc_bodies(&mut self, queue: &mut Vec<(String, bool)>) {
        for (delimiter, tabs) in queue.drain(..) {
            while self.at < self.source.len() {
                let start = self.at;
                while self.at < self.source.len() && self.source[self.at] != b'\n' {
                    self.advance();
                }
                let mut candidate = &self.source[start..self.at];
                if tabs {
                    while let Some(rest) = candidate.strip_prefix(b"\t") {
                        candidate = rest;
                    }
                }
                let done = candidate == delimiter.as_bytes();
                if self.at < self.source.len() {
                    self.advance();
                }
                if done {
                    break;
                }
            }
        }
    }

    fn word(&mut self, delimiter: bool, end: Option<u8>, depth: usize) -> Option<Word> {
        let start = self.at;
        let line = self.line;
        let mut text = Vec::new();
        let mut quote = None;
        let mut dynamic = false;
        let mut escaped = false;
        let mut plain = true;
        while let Some(&byte) = self.source.get(self.at) {
            if quote.is_none()
                && (byte.is_ascii_whitespace() || b";|&()< >".contains(&byte) || Some(byte) == end)
            {
                break;
            }
            if quote == Some(b'\'') {
                if byte == b'\'' {
                    quote = None;
                } else {
                    text.push(byte);
                }
                self.advance();
            } else if byte == b'\\' {
                self.advance();
                if let Some(&next) = self.source.get(self.at) {
                    if next == b'\n' {
                        self.advance();
                    } else if quote == Some(b'"') && !b"$`\"\\".contains(&next) {
                        text.push(b'\\');
                    } else {
                        text.push(next);
                        escaped |= !delimiter;
                        plain = false;
                        self.advance();
                    }
                } else {
                    dynamic = true;
                }
            } else if Some(byte) == quote {
                quote = None;
                self.advance();
            } else if quote.is_none() && matches!(byte, b'\'' | b'"') {
                plain = false;
                quote = Some(byte);
                self.advance();
            } else if !delimiter && self.starts(b"$((") {
                dynamic = true;
                self.at += 3;
                self.arithmetic(depth);
            } else if !delimiter && (self.starts(b"$(") || byte == b'`') {
                dynamic = true;
                let backtick = byte == b'`';
                let first = self.references.len();
                let substitution_line = self.line;
                self.at += if backtick { 1 } else { 2 };
                let closed = self.commands(
                    Some(if backtick { b'`' } else { b')' }),
                    depth + 1,
                    &mut Vec::new(),
                    false,
                );
                if backtick || !closed {
                    for (_, key) in &mut self.references[first..] {
                        *key = None;
                    }
                }
                if !closed {
                    self.unknown(substitution_line);
                }
            } else {
                dynamic |= !delimiter && matches!(byte, b'$' | b'*' | b'?' | b'[' | b'~');
                text.push(byte);
                self.advance();
            }
        }
        if quote.is_some() {
            self.unknown(line);
            dynamic = true;
        }
        (self.at != start).then(|| Word {
            text: String::from_utf8_lossy(&text).into_owned(),
            dynamic,
            escaped,
            plain: plain && !dynamic,
            assignment: Word::assignment(&self.source[start..self.at]),
            line,
        })
    }

    // Arithmetic text cannot be a command. Active nested substitutions still share
    // the lexer, but are conservatively unknown in this unsupported expression.
    fn arithmetic(&mut self, depth: usize) {
        let line = self.line;
        let mut nesting = 0usize;
        while let Some(&byte) = self.source.get(self.at) {
            if nesting == 0 && self.starts(b"))") {
                self.at += 2;
                return;
            }
            if self.starts(b"$(") && !self.starts(b"$((") {
                let first = self.references.len();
                self.at += 2;
                self.commands(Some(b')'), depth + 1, &mut Vec::new(), false);
                for (_, key) in &mut self.references[first..] {
                    *key = None;
                }
            } else if byte == b'\\' {
                self.advance();
                if self.at < self.source.len() {
                    self.advance();
                }
            } else {
                if byte == b'(' {
                    nesting += 1;
                } else if byte == b')' {
                    nesting = nesting.saturating_sub(1);
                }
                self.advance();
            }
        }
        self.unknown(line);
    }

    fn invocation(&mut self, words: &[Word], redirected: bool, data: bool) {
        if data {
            return;
        }
        let mut at = 0;
        let mut prefixed = redirected;
        let mut option_boundary = false;
        while let Some(word) = words.get(at) {
            let wrapper = !word.dynamic
                && matches!(
                    word.text.as_str(),
                    "command" | "env" | "exec" | "sudo" | "time"
                );
            let punctuation = word.plain
                && matches!(
                    word.text.as_str(),
                    "{" | "!" | "if" | "then" | "elif" | "else" | "while" | "until" | "do"
                );
            let option = prefixed && word.text.starts_with('-');
            option_boundary |= option;
            if word.assignment || wrapper || punctuation || option {
                prefixed = true;
                at += 1;
            } else {
                break;
            }
        }
        let Some(command) = words.get(at) else {
            return;
        };
        if command.text != "skillcfg" || command.dynamic {
            // ponytail: wrapper option operands are ambiguous without an option grammar.
            // Keep identified downstream invocations unknown, never evaluate their keys.
            if option_boundary {
                for candidate in &words[at..] {
                    if candidate.text == "skillcfg" && !candidate.dynamic {
                        self.unknown(candidate.line);
                    }
                }
            }
            return;
        }
        let rest = &words[at + 1..];
        let key = if !prefixed
            && !command.escaped
            && rest.len() == 2
            && rest[0].text == "get"
            && !rest.iter().any(|w| w.dynamic || w.escaped)
        {
            rest[1].text.parse().ok()
        } else {
            None
        };
        self.references.push((command.line, key));
    }
}
