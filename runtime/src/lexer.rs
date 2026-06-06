// Flux lexer — manba matnni tokenlar oqimiga aylantiradi.
//
// Eng muhim mas'uliyat: indentation'ni INDENT/DEDENT tokenlariga aylantirish.
// Flux bloklari 2-bo'shliqli chekinish bilan ochiladi (`{}` yo'q). Lexer har
// mazmunli qator boshida joriy chekinishni stack bilan solishtiradi:
//   - chuqurroq  -> Indent
//   - sayozroq   -> har pog'ona uchun bitta Dedent
//   - teng       -> hech narsa
// Bo'sh qatorlar va faqat-izoh qatorlar indentatsiyaga ta'sir qilmaydi.

use crate::token::{StrPart, Tok, Token};

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
    indents: Vec<usize>, // chekinish darajalari stacki (bo'shliq soni)
    tokens: Vec<Token>,
    // Qavs ichida ekanmizmi? Qavs ichida newline/indent e'tiborga olinmaydi
    // (ko'p qatorli list/map literallari uchun).
    paren_depth: usize,
    // Keyingi push qilinadigan token oldidan bo'shliq (yoki tab/newline) bormi.
    // Bo'shliq ko'rilganda true bo'ladi, token push'da o'qilib reset qilinadi.
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
            pending_space: true, // fayl boshi ham "bo'shliqdan keyin" kabi
        }
    }

    pub fn tokenize(mut self) -> LexResult<Vec<Token>> {
        // Faylni qatorma-qator emas, oqim sifatida o'qiymiz, lekin qator
        // boshida indentatsiyani aniqlaymiz.
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
                        self.push(Tok::Newline);
                        self.handle_line_start()?;
                    }
                }
                _ => self.scan_token()?,
            }
        }
        // Fayl oxiri: oxirgi newline va qolgan bloklarni yopish.
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

    // --- qator boshida indentatsiya hisoblash ---
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
                            "{}-qatorda tab ishlatilgan; Flux faqat bo'shliq qabul qiladi",
                            self.line
                        ));
                    }
                    _ => break,
                }
            }
            // Bo'sh yoki faqat-izoh qator: indentatsiyani e'tiborsiz qoldiramiz.
            if self.at_end() {
                return Ok(());
            }
            match self.peek() {
                b'\n' => {
                    self.advance_newline();
                    continue; // keyingi qatorni qayta tekshir
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
                    "{}-qatorda chekinish mos kelmadi (kutilgan darajalar: {:?}, topildi: {})",
                    self.line, self.indents, width
                ));
            }
            // Dedent statement chegarasi hamdir: blok yopilgandan keyin keyingi
            // qator yangi statement. Aks holda qavssiz chaqirish (juxtaposition
            // call) blok-tanali lambda/if'dan keyingi qatorni argument deb
            // yutib yuborardi (masalan ketma-ket `http.on ... \req ->` bloklari).
            self.push(Tok::Newline);
        }
        Ok(())
    }

    fn skip_comment(&mut self) {
        while !self.at_end() && self.peek() != b'\n' {
            self.advance();
        }
    }

    // --- bitta tokenni o'qish ---
    fn scan_token(&mut self) -> LexResult<()> {
        let line = self.line;
        let col = self.col;
        let c = self.peek();
        match c {
            b'0'..=b'9' => return self.scan_number(),
            b'"' => return self.scan_string(),
            b':' => {
                // Noaniqlik: `:open` symbolmi yoki `key:` map ajratuvchimi?
                // Qoida (misollardan): agar `:` BEVOSITA atom oxiriga (ident/son/
                // `)`/`]`/`"`) yopishgan bo'lsa — bu map ajratuvchi (Colon).
                // Aks holda (oldin bo'shliq, `(`, `[`, `,` yoki qator boshi) va
                // keyin ident kelsa — symbol. `status::open` -> Colon + Sym.
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

        // Operatorlar va punktuatsiya — ko'p belgilarini avval tekshiramiz.
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
                    return Err(format!("{}-qatorda kutilmagan '?'", line));
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
                    "{}-qatorda kutilmagan belgi: '{}'",
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
        // float nuqtasi, lekin '..' (range) emas
        if !self.at_end()
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
                .map_err(|_| format!("{}-qatorda noto'g'ri float: {}", line, text))?;
            self.push_at(Tok::Flt(v), line, col);
        } else {
            let v: i64 = text
                .parse()
                .map_err(|_| format!("{}-qatorda juda katta son: {}", line, text))?;
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

    // String literal: oddiy matn + ${expr} / $ident interpolatsiya.
    fn scan_string(&mut self) -> LexResult<()> {
        let line = self.line;
        let col = self.col;
        self.advance(); // ochuvchi "
        let mut parts: Vec<StrPart> = Vec::new();
        let mut buf = String::new();
        loop {
            if self.at_end() {
                return Err(format!("{}-qatorda yopilmagan satr", line));
            }
            let c = self.peek();
            match c {
                b'"' => {
                    self.advance();
                    break;
                }
                b'\\' => {
                    self.advance();
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
                b'$' => {
                    self.advance();
                    if self.peek_or(0) == b'{' {
                        // ${ ifoda }
                        self.advance(); // {
                        if !buf.is_empty() {
                            parts.push(StrPart::Lit(std::mem::take(&mut buf)));
                        }
                        let mut depth = 1;
                        let estart = self.pos;
                        while !self.at_end() && depth > 0 {
                            match self.peek() {
                                b'{' => depth += 1,
                                b'}' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            self.advance();
                        }
                        let expr = std::str::from_utf8(&self.src[estart..self.pos])
                            .unwrap()
                            .to_string();
                        self.advance(); // yopuvchi }
                        parts.push(StrPart::Expr(expr));
                    } else if self.is_ident_start(self.peek_or(0)) {
                        // $ident qisqartma
                        if !buf.is_empty() {
                            parts.push(StrPart::Lit(std::mem::take(&mut buf)));
                        }
                        let id = self.read_ident();
                        parts.push(StrPart::Expr(id));
                    } else {
                        buf.push('$');
                    }
                }
                b'\n' => {
                    return Err(format!("{}-qatorda satr ko'p qatorga cho'zildi", line));
                }
                _ => {
                    // UTF-8 xavfsiz: baytni to'g'ri belgiga yig'amiz
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

    // Joriy pozitsiyadagi bitta UTF-8 belgini buf ga qo'shadi.
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

    // --- past darajali yordamchilar ---
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
