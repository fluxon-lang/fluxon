// Fluxon lexer — turns source text into a token stream.
//
// Most important responsibility: turning indentation into INDENT/DEDENT tokens.
// Fluxon blocks open with 2-space indentation (no `{}`). At the start of each
// meaningful line the lexer compares the current indent against a stack:
//   - deeper   -> Indent
//   - shallower -> one Dedent per level popped
//   - equal    -> nothing
// Blank lines and comment-only lines do not affect indentation.

use crate::token::{StrPart, Tok, Token};

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
    indents: Vec<usize>, // stack of indent levels (space counts)
    tokens: Vec<Token>,
    // Are we inside parentheses? Inside them newline/indent are ignored
    // (for multi-line list/map literals).
    paren_depth: usize,
    // Whether whitespace (or a tab/newline) precedes the next token to be pushed.
    // Set to true when whitespace is seen; read and reset when a token is pushed.
    pending_space: bool,
}

pub type LexResult<T> = Result<T, String>;

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer {
            src: src.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
            indents: vec![0],
            tokens: Vec::new(),
            paren_depth: 0,
            pending_space: true, // start of file also counts as "after whitespace"
        }
    }

    pub fn tokenize(mut self) -> LexResult<Vec<Token>> {
        // We read the file as a stream rather than line by line, but detect
        // indentation at the start of each line.
        self.handle_line_start()?;
        while !self.at_end() {
            let c = self.peek();
            match c {
                b' ' | b'\t' | b'\r' => {
                    self.advance();
                    self.pending_space = true;
                }
                b'#' => {
                    self.skip_comment();
                    self.pending_space = true;
                }
                b'\n' => {
                    self.advance_newline();
                    self.pending_space = true;
                    if self.paren_depth == 0 {
                        // Line continuation: if the next line starts with `|>`,
                        // it is a continuation of the previous expression (a pipe
                        // chain) — we emit neither Newline nor INDENT, keeping the
                        // token stream contiguous. Builder chains span multiple
                        // lines (issue #78).
                        if self.next_line_starts_with_pipe() {
                            continue;
                        }
                        self.push(Tok::Newline);
                        self.handle_line_start()?;
                    }
                }
                _ => self.scan_token()?,
            }
        }
        // End of file: emit a final newline and close any remaining blocks.
        if !matches!(
            self.tokens.last().map(|t| &t.tok),
            Some(Tok::Newline) | None
        ) {
            self.push(Tok::Newline);
        }
        while self.indents.len() > 1 {
            self.indents.pop();
            self.push(Tok::Dedent);
        }
        self.push(Tok::Eof);
        Ok(self.tokens)
    }

    // --- compute indentation at the start of a line ---
    fn handle_line_start(&mut self) -> LexResult<()> {
        if self.paren_depth > 0 {
            return Ok(());
        }
        loop {
            let mut width = 0usize;
            let start = self.pos;
            while !self.at_end() {
                match self.peek() {
                    b' ' => {
                        width += 1;
                        self.advance();
                    }
                    b'\t' => {
                        return Err(format!(
                            "tab used on line {}; Fluxon only accepts spaces",
                            self.line
                        ));
                    }
                    _ => break,
                }
            }
            // Blank or comment-only line: ignore its indentation.
            if self.at_end() {
                return Ok(());
            }
            match self.peek() {
                b'\n' => {
                    self.advance_newline();
                    continue; // re-check the next line
                }
                b'#' => {
                    self.skip_comment();
                    if !self.at_end() && self.peek() == b'\n' {
                        self.advance_newline();
                        continue;
                    }
                    return Ok(());
                }
                _ => {}
            }
            let _ = start;
            self.emit_indentation(width)?;
            return Ok(());
        }
    }

    // Looking ahead from the current position (after a newline) — skipping
    // whitespace, blank lines and comments — does the next meaningful line start
    // with `|>`? Does NOT mutate position (only scans by index). Used to detect a
    // pipe continuation line (issue #78 builder chains).
    fn next_line_starts_with_pipe(&self) -> bool {
        let mut i = self.pos;
        loop {
            // leading whitespace on the current line
            while i < self.src.len() && (self.src[i] == b' ' || self.src[i] == b'\t') {
                i += 1;
            }
            if i >= self.src.len() {
                return false;
            }
            match self.src[i] {
                // blank line — move to the next one
                b'\n' => {
                    i += 1;
                    continue;
                }
                // comment line — skip to end of line, then to the next one
                b'#' => {
                    while i < self.src.len() && self.src[i] != b'\n' {
                        i += 1;
                    }
                    continue;
                }
                // start of a meaningful line: continuation if it is `|>`
                b'|' => return self.src.get(i + 1) == Some(&b'>'),
                _ => return false,
            }
        }
    }

    fn emit_indentation(&mut self, width: usize) -> LexResult<()> {
        let cur = *self.indents.last().unwrap();
        if width > cur {
            self.indents.push(width);
            self.push(Tok::Indent);
        } else if width < cur {
            while *self.indents.last().unwrap() > width {
                self.indents.pop();
                self.push(Tok::Dedent);
            }
            if *self.indents.last().unwrap() != width {
                return Err(format!(
                    "indentation mismatch on line {} (expected levels: {:?}, found: {})",
                    self.line, self.indents, width
                ));
            }
            // A Dedent is also a statement boundary: after a block closes, the
            // next line is a new statement. Otherwise a parenthesis-free
            // (juxtaposition) call would swallow the line after a block-bodied
            // lambda/if as an argument (e.g. consecutive `http.on ... \req ->`
            // blocks).
            self.push(Tok::Newline);
        }
        Ok(())
    }

    fn skip_comment(&mut self) {
        while !self.at_end() && self.peek() != b'\n' {
            self.advance();
        }
    }

    // --- scan a single token ---
    fn scan_token(&mut self) -> LexResult<()> {
        let line = self.line;
        let col = self.col;
        let c = self.peek();
        match c {
            b'0'..=b'9' => return self.scan_number(),
            b'"' => {
                // `"""` — multi-line block string (issue #130), otherwise a plain string.
                if self.peek_or(1) == b'"' && self.peek_or(2) == b'"' {
                    return self.scan_block_string();
                }
                return self.scan_string();
            }
            b':' => {
                // Ambiguity: is `:open` a symbol or `key:` a map separator?
                // Rule (from the examples): if `:` is glued DIRECTLY to the end of
                // an atom (ident/number/`)`/`]`/`"`) it is a map separator (Colon).
                // Otherwise (preceded by whitespace, `(`, `[`, `,` or start of
                // line) and followed by an ident — it is a symbol.
                // `status::open` -> Colon + Sym.
                let prev = if self.pos > 0 {
                    self.src[self.pos - 1]
                } else {
                    b' '
                };
                let glued_to_atom =
                    self.is_ident_cont(prev) || prev == b')' || prev == b']' || prev == b'"';
                self.advance();
                if !glued_to_atom && self.is_ident_start(self.peek_or(0)) {
                    let s = self.read_ident();
                    self.push_at(Tok::Sym(s), line, col);
                } else {
                    self.push_at(Tok::Colon, line, col);
                }
                return Ok(());
            }
            _ if self.is_ident_start(c) => return self.scan_ident(),
            _ => {}
        }

        // Operators and punctuation — check multi-char ones first.
        self.advance();
        let tok = match c {
            b'+' => Tok::Plus,
            b'-' => {
                if self.peek_or(0) == b'>' {
                    self.advance();
                    Tok::Arrow
                } else {
                    Tok::Minus
                }
            }
            b'*' => Tok::Star,
            b'/' => Tok::Slash,
            b'%' => Tok::Percent,
            b'=' => {
                if self.peek_or(0) == b'=' {
                    self.advance();
                    Tok::EqEq
                } else {
                    Tok::Eq
                }
            }
            b'!' => {
                if self.peek_or(0) == b'=' {
                    self.advance();
                    Tok::NotEq
                } else {
                    Tok::Bang
                }
            }
            b'<' => match self.peek_or(0) {
                b'-' => {
                    self.advance();
                    Tok::Assign
                }
                b'=' => {
                    self.advance();
                    Tok::LtEq
                }
                _ => Tok::Lt,
            },
            b'>' => {
                if self.peek_or(0) == b'=' {
                    self.advance();
                    Tok::GtEq
                } else {
                    Tok::Gt
                }
            }
            b'&' => Tok::Amp,
            b'|' => {
                if self.peek_or(0) == b'>' {
                    self.advance();
                    Tok::PipeGt
                } else {
                    Tok::Pipe
                }
            }
            b'?' => {
                if self.peek_or(0) == b'?' {
                    self.advance();
                    Tok::Question2
                } else {
                    return Err(format!("unexpected '?' on line {}", line));
                }
            }
            b'.' => {
                if self.peek_or(0) == b'.' && self.peek_or(1) == b'.' {
                    self.advance();
                    self.advance();
                    Tok::Spread
                } else if self.peek_or(0) == b'.' {
                    self.advance();
                    Tok::DotDot
                } else {
                    Tok::Dot
                }
            }
            b'\\' => Tok::Backslash,
            b'(' => {
                self.paren_depth += 1;
                Tok::LParen
            }
            b')' => {
                self.paren_depth = self.paren_depth.saturating_sub(1);
                Tok::RParen
            }
            b'[' => {
                self.paren_depth += 1;
                Tok::LBracket
            }
            b']' => {
                self.paren_depth = self.paren_depth.saturating_sub(1);
                Tok::RBracket
            }
            b'{' => {
                self.paren_depth += 1;
                Tok::LBrace
            }
            b'}' => {
                self.paren_depth = self.paren_depth.saturating_sub(1);
                Tok::RBrace
            }
            b',' => Tok::Comma,
            _ => {
                return Err(format!(
                    "unexpected character on line {}: '{}'",
                    line, c as char
                ));
            }
        };
        self.push_at(tok, line, col);
        Ok(())
    }

    fn scan_number(&mut self) -> LexResult<()> {
        let line = self.line;
        let col = self.col;
        let start = self.pos;
        while !self.at_end() && self.peek().is_ascii_digit() {
            self.advance();
        }
        let mut is_float = false;
        // member-index context: in `m.0.1` we do not swallow `.1` as a float
        // fraction — if the previous token is `.`, this number is an index
        // (`(m.0).1`), not a float.
        let after_dot = matches!(self.tokens.last(), Some(t) if t.tok == Tok::Dot);
        // a float dot, but not '..' (range)
        if !after_dot
            && !self.at_end()
            && self.peek() == b'.'
            && self.peek_or(1) != b'.'
            && self.peek_or(1).is_ascii_digit()
        {
            is_float = true;
            self.advance(); // '.'
            while !self.at_end() && self.peek().is_ascii_digit() {
                self.advance();
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
        if is_float {
            let v: f64 = text
                .parse()
                .map_err(|_| format!("invalid float on line {}: {}", line, text))?;
            self.push_at(Tok::Flt(v), line, col);
        } else {
            let v: i64 = text
                .parse()
                .map_err(|_| format!("number too large on line {}: {}", line, text))?;
            self.push_at(Tok::Int(v), line, col);
        }
        Ok(())
    }

    fn scan_ident(&mut self) -> LexResult<()> {
        let line = self.line;
        let col = self.col;
        let s = self.read_ident();
        let tok = match s.as_str() {
            "true" => Tok::True,
            "false" => Tok::False,
            "nil" => Tok::Nil,
            "inf" => Tok::Inf,
            "fn" => Tok::Fn,
            "ret" => Tok::Ret,
            "if" => Tok::If,
            "elif" => Tok::Elif,
            "else" => Tok::Else,
            "each" => Tok::Each,
            "in" => Tok::In,
            "match" => Tok::Match,
            "skip" => Tok::Skip,
            "stop" => Tok::Stop,
            "use" => Tok::Use,
            "exp" => Tok::Exp,
            "as" => Tok::As,
            "tbl" => Tok::Tbl,
            "fail" => Tok::Fail,
            "try" => Tok::Try,
            "catch" => Tok::Catch,
            _ => Tok::Ident(s),
        };
        self.push_at(tok, line, col);
        Ok(())
    }

    fn read_ident(&mut self) -> String {
        let start = self.pos;
        while !self.at_end() && self.is_ident_cont(self.peek()) {
            self.advance();
        }
        std::str::from_utf8(&self.src[start..self.pos])
            .unwrap()
            .to_string()
    }

    // String literal: plain text + ${expr} / $ident interpolation.
    fn scan_string(&mut self) -> LexResult<()> {
        let line = self.line;
        let col = self.col;
        self.advance(); // opening "
        let mut parts: Vec<StrPart> = Vec::new();
        let mut buf = String::new();
        loop {
            if self.at_end() {
                return Err(format!("unterminated string on line {}", line));
            }
            let c = self.peek();
            match c {
                b'"' => {
                    self.advance();
                    break;
                }
                b'\\' => self.scan_escape(&mut buf),
                b'$' => self.scan_dollar(&mut buf, &mut parts),
                b'\n' => {
                    return Err(format!("string spans multiple lines on line {}", line));
                }
                _ => {
                    // UTF-8 safe: accumulate the byte into a proper character
                    self.push_utf8_char(&mut buf);
                }
            }
        }
        if !buf.is_empty() || parts.is_empty() {
            parts.push(StrPart::Lit(buf));
        }
        self.push_at(Tok::Str(parts), line, col);
        Ok(())
    }

    // Expands a `\x` escape sequence in a string into buf.
    fn scan_escape(&mut self, buf: &mut String) {
        self.advance(); // '\'
        let e = self.peek_or(0);
        self.advance();
        match e {
            b'n' => buf.push('\n'),
            b't' => buf.push('\t'),
            b'r' => buf.push('\r'),
            b'"' => buf.push('"'),
            b'\\' => buf.push('\\'),
            b'$' => buf.push('$'),
            _ => {
                buf.push('\\');
                buf.push(e as char);
            }
        }
    }

    // Handles a `$` inside a string: `${expr}` / `$ident` interpolation,
    // otherwise a plain `$` character. Shared by plain and block strings.
    fn scan_dollar(&mut self, buf: &mut String, parts: &mut Vec<StrPart>) {
        let expr_line = self.line;
        self.advance();
        if self.peek_or(0) == b'{' {
            // ${ expr }
            self.advance(); // {
            if !buf.is_empty() {
                parts.push(StrPart::Lit(std::mem::take(buf)));
            }
            // When finding the boundary we account for inner string literals —
            // otherwise a `}` inside the string in `${"a } b"}` would close the
            // interpolation early (issue #106).
            let mut depth = 1;
            let mut in_str = false;
            let estart = self.pos;
            while !self.at_end() && depth > 0 {
                let c = self.peek();
                if in_str {
                    match c {
                        // escape: `\` + the next char are consumed together,
                        // so a `\"` inside the string is not a closing quote.
                        b'\\' => {
                            self.advance();
                            if self.peek_or(0) == b'\n' {
                                self.advance_newline();
                            } else if !self.at_end() {
                                self.advance();
                            }
                            continue;
                        }
                        b'"' => in_str = false,
                        _ => {}
                    }
                } else {
                    match c {
                        b'"' => in_str = true,
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                // In a multi-line `${...}` we keep the line count accurate.
                if c == b'\n' {
                    self.advance_newline();
                } else {
                    self.advance();
                }
            }
            let expr = std::str::from_utf8(&self.src[estart..self.pos])
                .unwrap()
                .to_string();
            self.advance(); // closing }
            parts.push(StrPart::Expr(expr, expr_line));
        } else if self.is_ident_start(self.peek_or(0)) {
            // $ident shorthand
            if !buf.is_empty() {
                parts.push(StrPart::Lit(std::mem::take(buf)));
            }
            let id = self.read_ident();
            parts.push(StrPart::Expr(id, expr_line));
        } else {
            buf.push('$');
        }
    }

    // Multi-line block string: `"""` ... `"""` (issue #130).
    //
    // Rules (the one canonical way):
    //   - no content on the line after the opening `"""` — it starts on a new line;
    //   - the smallest common indentation of non-blank lines is stripped
    //     (so the block fits naturally into the surrounding code indentation);
    //   - if the closing `"""` is on its own line, that line is not part of the
    //     content — so there is no trailing `\n`;
    //   - `${expr}` / `$ident` interpolation and `\x` escapes work as in plain strings;
    //   - `"` and `""` may be written freely; for three consecutive `"` use `\"""`.
    fn scan_block_string(&mut self) -> LexResult<()> {
        let line = self.line;
        let col = self.col;
        self.advance(); // "
        self.advance(); // "
        self.advance(); // "
        // The rest of the opening `"""` line may only be whitespace.
        while !self.at_end() && (self.peek() == b' ' || self.peek() == b'\r') {
            self.advance();
        }
        if self.at_end() || self.peek() != b'\n' {
            return Err(format!(
                "text after \"\"\" on line {} — block string content starts on a new line",
                line
            ));
        }
        self.advance_newline();

        // Each line is collected separately so indentation can be stripped later.
        // A `\n` introduced via an escape stays inside a Lit and does not count
        // as a line break.
        let mut lines: Vec<Vec<StrPart>> = Vec::new();
        let mut parts: Vec<StrPart> = Vec::new();
        let mut buf = String::new();
        loop {
            if self.at_end() {
                return Err(format!(
                    "unterminated block string on line {} (missing \"\"\")",
                    line
                ));
            }
            match self.peek() {
                b'"' if self.peek_or(1) == b'"' && self.peek_or(2) == b'"' => {
                    self.advance();
                    self.advance();
                    self.advance();
                    break;
                }
                b'\n' => {
                    self.advance_newline();
                    if !buf.is_empty() {
                        parts.push(StrPart::Lit(std::mem::take(&mut buf)));
                    }
                    lines.push(std::mem::take(&mut parts));
                }
                // In CRLF files the `\r` is a line-ending marker — not content.
                b'\r' if self.peek_or(1) == b'\n' => self.advance(),
                b'\\' => self.scan_escape(&mut buf),
                b'$' => self.scan_dollar(&mut buf, &mut parts),
                _ => self.push_utf8_char(&mut buf),
            }
        }
        if !buf.is_empty() {
            parts.push(StrPart::Lit(buf));
        }
        lines.push(parts);

        self.push_at(Tok::Str(dedent_block_lines(lines)), line, col);
        Ok(())
    }

    // Appends the single UTF-8 character at the current position into buf.
    fn push_utf8_char(&mut self, buf: &mut String) {
        let b = self.peek();
        let len = utf8_len(b);
        let end = (self.pos + len).min(self.src.len());
        if let Ok(s) = std::str::from_utf8(&self.src[self.pos..end]) {
            buf.push_str(s);
        }
        for _ in 0..len {
            self.advance();
        }
    }

    // --- low-level helpers ---
    fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }
    fn peek(&self) -> u8 {
        self.src[self.pos]
    }
    fn peek_or(&self, ahead: usize) -> u8 {
        *self.src.get(self.pos + ahead).unwrap_or(&0)
    }
    fn advance(&mut self) {
        self.pos += 1;
        self.col += 1;
    }
    fn advance_newline(&mut self) {
        self.pos += 1;
        self.line += 1;
        self.col = 1;
    }
    fn is_ident_start(&self, c: u8) -> bool {
        c == b'_' || c.is_ascii_alphabetic()
    }
    fn is_ident_cont(&self, c: u8) -> bool {
        c == b'_' || c.is_ascii_alphanumeric()
    }
    fn push(&mut self, tok: Tok) {
        let line = self.line;
        let col = self.col;
        self.push_at(tok, line, col);
    }
    fn push_at(&mut self, tok: Tok, line: usize, col: usize) {
        let spaced = self.pending_space;
        self.pending_space = false;
        self.tokens.push(Token {
            tok,
            line,
            col,
            spaced,
        });
    }
}

// Assembles block-string lines into the final StrPart stream: drops the closing
// `"""` line, strips the common minimal indentation, and joins lines with `\n`.
// Blank (whitespace-only) lines become `\n` — their indentation does not reach
// the output and is not counted in the minimum.
fn dedent_block_lines(mut lines: Vec<Vec<StrPart>>) -> Vec<StrPart> {
    fn is_blank(line: &[StrPart]) -> bool {
        match line {
            [] => true,
            [StrPart::Lit(s)] => s.chars().all(|c| c == ' '),
            _ => false,
        }
    }
    // If the closing `"""` is on its own line, the last "line" consists only of
    // its indentation — not content (and there is no trailing `\n`).
    if lines.last().is_some_and(|l| is_blank(l)) {
        lines.pop();
    }
    // Minimum indent: if a line starts with a Lit, it is the count of leading
    // spaces; if it starts with interpolation, it is 0 (meaning no source indent).
    let mut min_indent = usize::MAX;
    for l in &lines {
        if is_blank(l) {
            continue;
        }
        let ind = match l.first() {
            Some(StrPart::Lit(s)) => s.chars().take_while(|&c| c == ' ').count(),
            _ => 0,
        };
        min_indent = min_indent.min(ind);
    }
    if min_indent == usize::MAX {
        min_indent = 0;
    }

    let mut out: Vec<StrPart> = Vec::new();
    let mut buf = String::new();
    for (i, l) in lines.into_iter().enumerate() {
        if i > 0 {
            buf.push('\n');
        }
        if is_blank(&l) {
            continue;
        }
        for (j, p) in l.into_iter().enumerate() {
            match p {
                StrPart::Lit(mut s) => {
                    if j == 0 {
                        // only leading spaces — ASCII, so the byte boundary is safe
                        s.drain(..min_indent);
                    }
                    buf.push_str(&s);
                }
                e @ StrPart::Expr(..) => {
                    if !buf.is_empty() {
                        out.push(StrPart::Lit(std::mem::take(&mut buf)));
                    }
                    out.push(e);
                }
            }
        }
    }
    if !buf.is_empty() || out.is_empty() {
        out.push(StrPart::Lit(buf));
    }
    out
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else if b >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

pub fn lex(src: &str) -> LexResult<Vec<Token>> {
    Lexer::new(src).tokenize()
}

// Lexes the source starting from the given line. Used when re-lexing string
// interpolation expressions — the emitted tokens (and any errors derived from
// them) preserve the original line number.
pub fn lex_at(src: &str, start_line: usize) -> LexResult<Vec<Token>> {
    let mut lexer = Lexer::new(src);
    lexer.line = start_line;
    lexer.tokenize()
}
