// Flux parser — tokenlardan AST quradi.
//
// Ikki qatlam:
//   1) Statement/blok darajasi (recursive descent): Indent/Dedent/Newline
//      tokenlariga tayanadi.
//   2) Expression darajasi (precedence climbing): operatorlar ustuvorligi.
//
// Eng nozik joy — QAVSSIZ CHAQIRISH. Flux'da `f a b` = chaqiruv, `a + b` =
// operator. Yechim: eng yuqori ("application") darajada ketma-ket "atom"larni
// yig'amiz; agar bittadan ortiq atom yonma-yon kelsa, birinchisi callee,
// qolganlari argument. Atom — operator yoki blok-chegara tokeniga duch
// kelguncha o'qiladigan eng kichik to'liq ifoda.

use crate::ast::*;
use crate::token::{StrPart, Tok, Token};

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    // List/map literal ichida qavssiz (juxtaposition) chaqiruv ishlatilmaydi —
    // u yerda har element atom yoki qavsli chaqiruv. Bu bayroq shu kontekstda
    // application bosqichini o'chiradi, shunda `{a:f b:g}` da `f` `b`ni argument
    // sifatida yutmaydi. Chaqiruv kerak bo'lsa: `{a:(f x)}`.
    no_app: bool,
}

pub type ParseResult<T> = Result<T, String>;

pub fn parse(toks: Vec<Token>) -> ParseResult<Program> {
    let mut p = Parser {
        toks,
        pos: 0,
        no_app: false,
    };
    p.parse_program()
}

impl Parser {
    // --- token oqimi yordamchilari ---
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn peek2(&self) -> &Tok {
        self.toks
            .get(self.pos + 1)
            .map(|t| &t.tok)
            .unwrap_or(&Tok::Eof)
    }
    fn line(&self) -> usize {
        self.toks[self.pos].line
    }
    // Joriy token oldidan bo'shliq bormi (grammatik ajratish uchun).
    fn spaced(&self) -> bool {
        self.toks[self.pos].spaced
    }
    fn advance(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn check(&self, t: &Tok) -> bool {
        self.peek() == t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.check(t) {
            self.advance();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, t: &Tok, what: &str) -> ParseResult<()> {
        if self.check(t) {
            self.advance();
            Ok(())
        } else {
            Err(format!(
                "{}-qatorda {} kutilgan edi, lekin {:?} topildi",
                self.line(),
                what,
                self.peek()
            ))
        }
    }
    // Statement chegarasidagi newline'larni yutadi.
    fn skip_newlines(&mut self) {
        while self.check(&Tok::Newline) {
            self.advance();
        }
    }

    // --- dastur ---
    fn parse_program(&mut self) -> ParseResult<Program> {
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.check(&Tok::Eof) {
            stmts.push(self.parse_stmt()?);
            self.skip_newlines();
        }
        Ok(stmts)
    }

    // Indent...Dedent bilan o'ralgan blok. Chaqiruvchi avval Newline'ni yutib,
    // Indent kelishini ta'minlaydi.
    fn parse_block(&mut self) -> ParseResult<Vec<Stmt>> {
        self.expect(&Tok::Indent, "blok (chekinish)")?;
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.check(&Tok::Dedent) && !self.check(&Tok::Eof) {
            stmts.push(self.parse_stmt()?);
            self.skip_newlines();
        }
        self.expect(&Tok::Dedent, "blok oxiri")?;
        Ok(stmts)
    }

    // `->` dan keyingi tana: bir qatorli ifoda YOKI yangi qatordagi blok.
    fn parse_arrow_body(&mut self) -> ParseResult<Vec<Stmt>> {
        if self.check(&Tok::Newline) {
            self.advance();
            self.parse_block()
        } else {
            // bir qatorli: bitta ifoda
            let e = self.parse_expr()?;
            Ok(vec![Stmt::Expr(e)])
        }
    }

    // --- statementlar ---
    fn parse_stmt(&mut self) -> ParseResult<Stmt> {
        match self.peek() {
            Tok::Fn => self.parse_fn(false),
            Tok::Exp => self.parse_exp(),
            Tok::If => Ok(Stmt::Expr(self.parse_if()?)),
            Tok::Match => Ok(Stmt::Expr(self.parse_match()?)),
            Tok::Each => self.parse_each(),
            Tok::Ret => {
                self.advance();
                if self.check(&Tok::Newline) || self.check(&Tok::Dedent) || self.check(&Tok::Eof) {
                    Ok(Stmt::Ret(None))
                } else {
                    Ok(Stmt::Ret(Some(self.parse_expr()?)))
                }
            }
            Tok::Skip => {
                self.advance();
                Ok(Stmt::Skip)
            }
            Tok::Stop => {
                self.advance();
                Ok(Stmt::Stop)
            }
            Tok::Fail => self.parse_fail(),
            Tok::Use => self.parse_use(),
            Tok::Tbl => self.parse_tbl(),
            Tok::Ident(_) => self.parse_ident_stmt(),
            _ => {
                // boshqa har qanday ifoda statement
                let e = self.parse_expr()?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    // Ident bilan boshlangan: bind (=), assign (<-), yoki chaqiruv ifodasi.
    fn parse_ident_stmt(&mut self) -> ParseResult<Stmt> {
        // bir token oldinga qarab `name =` yoki `name <-` ni aniqlaymiz.
        if let Tok::Ident(name) = self.peek().clone() {
            match self.peek2() {
                Tok::Eq => {
                    self.advance(); // name
                    self.advance(); // =
                    let value = self.parse_expr()?;
                    return Ok(Stmt::Bind { name, value });
                }
                Tok::Assign => {
                    self.advance(); // name
                    self.advance(); // <-
                    let value = self.parse_expr()?;
                    return Ok(Stmt::Assign { name, value });
                }
                _ => {}
            }
        }
        let e = self.parse_expr()?;
        Ok(Stmt::Expr(e))
    }

    fn parse_fn(&mut self, exported: bool) -> ParseResult<Stmt> {
        self.advance(); // fn
        let name = self.expect_ident("funksiya nomi")?;
        let mut params = Vec::new();
        while let Tok::Ident(_) = self.peek() {
            let p = self.expect_ident("parametr")?;
            if params.contains(&p) {
                return Err(format!("'{}' funksiyasida takror parametr nomi: '{}'", name, p));
            }
            params.push(p);
        }
        if self.eat(&Tok::Arrow) {
            // bir qatorli: fn double x -> x * 2
            let body = self.parse_arrow_body()?;
            Ok(Stmt::FnDecl {
                name,
                params,
                body,
                exported,
            })
        } else {
            self.expect(&Tok::Newline, "funksiya tanasi")?;
            let body = self.parse_block()?;
            Ok(Stmt::FnDecl {
                name,
                params,
                body,
                exported,
            })
        }
    }

    fn parse_exp(&mut self) -> ParseResult<Stmt> {
        self.advance(); // exp
        if self.check(&Tok::Fn) {
            return self.parse_fn(true);
        }
        // exp NAME = expr
        let name = self.expect_ident("eksport nomi")?;
        self.expect(&Tok::Eq, "'='")?;
        let value = self.parse_expr()?;
        Ok(Stmt::ExpBind { name, value })
    }

    fn parse_each(&mut self) -> ParseResult<Stmt> {
        self.advance(); // each
        let mut vars = vec![self.expect_ident("loop o'zgaruvchisi")?];
        if self.eat(&Tok::Comma) {
            vars.push(self.expect_ident("ikkinchi loop o'zgaruvchisi")?);
        }
        self.expect(&Tok::In, "'in'")?;
        let iter = self.parse_expr()?;
        self.expect(&Tok::Newline, "each tanasi")?;
        let body = self.parse_block()?;
        Ok(Stmt::Each { vars, iter, body })
    }

    fn parse_fail(&mut self) -> ParseResult<Stmt> {
        let e = self.parse_fail_expr()?;
        Ok(Stmt::Expr(e))
    }

    // `fail [status] message` — ifoda sifatida (statement ham shuni ishlatadi).
    // `fail`dan keyingi argumentlar qavssiz application kabi yig'iladi.
    fn parse_fail_expr(&mut self) -> ParseResult<Expr> {
        self.advance(); // fail
        let first = self.parse_postfix()?;
        // fail 422 "xabar"  -> status + message ;  fail "xabar" -> faqat message
        if self.is_atom_start() {
            let message = self.parse_postfix()?;
            Ok(Expr::Fail {
                status: Some(Box::new(first)),
                message: Box::new(message),
            })
        } else {
            Ok(Expr::Fail {
                status: None,
                message: Box::new(first),
            })
        }
    }

    fn parse_use(&mut self) -> ParseResult<Stmt> {
        self.advance(); // use
        let mut items = Vec::new();
        loop {
            let path = match self.peek().clone() {
                Tok::Ident(s) => {
                    self.advance();
                    s
                }
                // ./tools  ->  Slash? aslida lexer'da './tools' qanday chiqadi?
                // './tools' = Dot Slash Ident. Buni yig'amiz.
                Tok::Dot => self.parse_module_path()?,
                _ => break,
            };
            let alias = if self.eat(&Tok::As) {
                Some(self.expect_ident("alias nomi")?)
            } else {
                None
            };
            items.push(UseItem { path, alias });
            if self.check(&Tok::Newline) || self.check(&Tok::Eof) {
                break;
            }
        }
        Ok(Stmt::Use { items })
    }

    // ./tools  yoki  ../lib/x  kabi modul yo'lini yig'adi.
    fn parse_module_path(&mut self) -> ParseResult<String> {
        let mut s = String::new();
        loop {
            match self.peek() {
                Tok::Dot => {
                    s.push('.');
                    self.advance();
                }
                Tok::DotDot => {
                    s.push_str("..");
                    self.advance();
                }
                Tok::Slash => {
                    s.push('/');
                    self.advance();
                }
                Tok::Ident(id) => {
                    s.push_str(id);
                    self.advance();
                }
                _ => break,
            }
        }
        Ok(s)
    }

    fn parse_tbl(&mut self) -> ParseResult<Stmt> {
        self.advance(); // tbl
        let name = self.expect_ident("jadval nomi")?;
        self.expect(&Tok::Newline, "jadval tanasi")?;
        self.expect(&Tok::Indent, "jadval ustunlari (chekinish)")?;
        let mut columns = Vec::new();
        self.skip_newlines();
        while !self.check(&Tok::Dedent) && !self.check(&Tok::Eof) {
            // ustun:  nom tip mod1 mod2...
            // yoki:   uniq(a, b)  — ko'p ustunli unikal (yadroda e'tiborsiz)
            let col_name = self.expect_ident("ustun nomi")?;
            let mut modifiers = Vec::new();
            let mut type_name = String::new();
            if let Tok::Ident(_) = self.peek() {
                type_name = self.expect_ident("ustun tipi")?;
            }
            while let Tok::Ident(m) = self.peek().clone() {
                self.advance();
                modifiers.push(m);
            }
            // ref:tbl.col yoki uniq(...) kabilarni qator oxirigacha o'tkazib yuboramiz
            while !self.check(&Tok::Newline) && !self.check(&Tok::Dedent) && !self.check(&Tok::Eof)
            {
                self.advance();
            }
            columns.push(TblColumn {
                name: col_name,
                type_name,
                modifiers,
            });
            self.skip_newlines();
        }
        self.expect(&Tok::Dedent, "jadval oxiri")?;
        Ok(Stmt::Tbl { name, columns })
    }

    // --- ifodalar (precedence climbing) ---
    fn parse_expr(&mut self) -> ParseResult<Expr> {
        self.parse_binary(0)
    }

    // Operator ustuvorligi jadvali. Kichik raqam = past ustuvorlik.
    fn bin_prec(t: &Tok) -> Option<(BinOp, u8)> {
        Some(match t {
            Tok::Pipe => (BinOp::Or, 1),
            Tok::Amp => (BinOp::And, 2),
            Tok::EqEq => (BinOp::Eq, 3),
            Tok::NotEq => (BinOp::Ne, 3),
            Tok::Lt => (BinOp::Lt, 4),
            Tok::LtEq => (BinOp::Le, 4),
            Tok::Gt => (BinOp::Gt, 4),
            Tok::GtEq => (BinOp::Ge, 4),
            Tok::Question2 => (BinOp::Coalesce, 5),
            Tok::PipeGt => (BinOp::Pipe, 6),
            Tok::Plus => (BinOp::Add, 7),
            Tok::Minus => (BinOp::Sub, 7),
            Tok::Star => (BinOp::Mul, 8),
            Tok::Slash => (BinOp::Div, 8),
            Tok::Percent => (BinOp::Mod, 8),
            _ => return None,
        })
    }

    fn parse_binary(&mut self, min_prec: u8) -> ParseResult<Expr> {
        let mut lhs = self.parse_range()?;
        while let Some((op, prec)) = Self::bin_prec(self.peek()) {
            if prec < min_prec {
                break;
            }
            self.advance();
            // chap-assotsiativ: o'ng tomon yuqoriroq ustuvorlik bilan
            let rhs = self.parse_binary(prec + 1)?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    // Range `a..b` — binary'dan yuqori, application'dan past.
    fn parse_range(&mut self) -> ParseResult<Expr> {
        let lhs = self.parse_application()?;
        if self.check(&Tok::DotDot) {
            self.advance();
            let rhs = self.parse_application()?;
            return Ok(Expr::Range {
                start: Box::new(lhs),
                end: Box::new(rhs),
            });
        }
        Ok(lhs)
    }

    // Qavssiz chaqirish: atom atom atom...
    fn parse_application(&mut self) -> ParseResult<Expr> {
        let first = self.parse_postfix()?;
        // List/map literal ichida juxtaposition-call o'chirilgan.
        if self.no_app {
            return Ok(first);
        }
        // Keyingi token yana atom boshlasa — bu chaqiruv.
        if !self.is_atom_start() {
            return Ok(first);
        }
        let mut args = Vec::new();
        while self.is_atom_start() {
            args.push(self.parse_postfix()?);
        }
        Ok(Expr::Call {
            callee: Box::new(first),
            args,
        })
    }

    // postfix: .field, [index], ! (try)
    fn parse_postfix(&mut self) -> ParseResult<Expr> {
        let mut e = self.parse_primary()?;
        loop {
            match self.peek() {
                Tok::Dot => {
                    self.advance();
                    // .name  yoki  .0 (raqamli indeks)
                    match self.peek().clone() {
                        Tok::Ident(name) => {
                            self.advance();
                            e = Expr::Field {
                                target: Box::new(e),
                                name,
                            };
                        }
                        Tok::Int(n) => {
                            self.advance();
                            e = Expr::Index {
                                target: Box::new(e),
                                key: Box::new(Expr::Int(n)),
                            };
                        }
                        other => {
                            return Err(format!(
                                "{}-qatorda '.' dan keyin nom yoki indeks kutilgan, {:?} topildi",
                                self.line(),
                                other
                            ));
                        }
                    }
                }
                // `[` postfix indeks BO'LADI faqat tutash bo'lsa (`arr[i]`).
                // Bo'shliq bilan kelsa (`f "x" [a]`) bu alohida list argument —
                // parse_application uni o'zi oladi.
                Tok::LBracket if !self.spaced() => {
                    self.advance();
                    let key = self.parse_expr()?;
                    self.expect(&Tok::RBracket, "']'")?;
                    e = Expr::Index {
                        target: Box::new(e),
                        key: Box::new(key),
                    };
                }
                Tok::Bang => {
                    self.advance();
                    e = Expr::Try(Box::new(e));
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> ParseResult<Expr> {
        match self.peek().clone() {
            Tok::Int(n) => {
                self.advance();
                Ok(Expr::Int(n))
            }
            Tok::Flt(f) => {
                self.advance();
                Ok(Expr::Flt(f))
            }
            Tok::True => {
                self.advance();
                Ok(Expr::Bool(true))
            }
            Tok::False => {
                self.advance();
                Ok(Expr::Bool(false))
            }
            Tok::Nil => {
                self.advance();
                Ok(Expr::Nil)
            }
            Tok::Sym(s) => {
                self.advance();
                Ok(Expr::Sym(s))
            }
            Tok::Ident(s) => {
                self.advance();
                Ok(Expr::Ident(s))
            }
            Tok::Str(parts) => {
                self.advance();
                self.build_string(parts)
            }
            Tok::Minus => {
                self.advance();
                let e = self.parse_postfix()?;
                Ok(Expr::Unary {
                    op: UnOp::Neg,
                    expr: Box::new(e),
                })
            }
            Tok::Bang => {
                self.advance();
                let e = self.parse_postfix()?;
                Ok(Expr::Unary {
                    op: UnOp::Not,
                    expr: Box::new(e),
                })
            }
            Tok::LParen => {
                self.advance();
                // Qavs ichida to'liq application yana yoqiladi.
                let saved = self.no_app;
                self.no_app = false;
                let e = self.parse_expr()?;
                self.no_app = saved;
                self.expect(&Tok::RParen, "')'")?;
                Ok(e)
            }
            Tok::LBracket => self.parse_list(),
            Tok::LBrace => self.parse_map(),
            Tok::Backslash => self.parse_lambda(),
            Tok::If => self.parse_if(),
            Tok::Match => self.parse_match(),
            Tok::Fail => self.parse_fail_expr(),
            other => Err(format!(
                "{}-qatorda ifoda kutilgan, {:?} topildi",
                self.line(),
                other
            )),
        }
    }

    fn build_string(&mut self, parts: Vec<StrPart>) -> ParseResult<Expr> {
        let mut pieces = Vec::new();
        for p in parts {
            match p {
                StrPart::Lit(s) => pieces.push(StrPiece::Lit(s)),
                StrPart::Expr(src) => {
                    // ifoda manbasini mustaqil tokenize + parse qilamiz
                    let toks = crate::lexer::lex(&src)
                        .map_err(|e| format!("interpolatsiya ichida: {}", e))?;
                    let mut sub = Parser {
                        toks,
                        pos: 0,
                        no_app: false,
                    };
                    sub.skip_newlines();
                    let e = sub.parse_expr()?;
                    pieces.push(StrPiece::Expr(e));
                }
            }
        }
        Ok(Expr::Str(pieces))
    }

    fn parse_list(&mut self) -> ParseResult<Expr> {
        self.advance(); // [
        let saved = self.no_app;
        self.no_app = true; // elementlar bo'shliq bilan ajraladi (juxtaposition-call yo'q)
        let mut items = Vec::new();
        self.skip_newlines();
        while !self.check(&Tok::RBracket) && !self.check(&Tok::Eof) {
            items.push(self.parse_expr()?);
            self.eat(&Tok::Comma); // vergul ixtiyoriy/tolerantlik
            self.skip_newlines();
        }
        self.no_app = saved;
        self.expect(&Tok::RBracket, "']'")?;
        Ok(Expr::List(items))
    }

    fn parse_map(&mut self) -> ParseResult<Expr> {
        self.advance(); // {
        let saved = self.no_app;
        self.no_app = true; // qiymatlar atom/qavsli; `{a:f b:g}` da f g ni yutmaydi
        let mut entries = Vec::new();
        self.skip_newlines();
        while !self.check(&Tok::RBrace) && !self.check(&Tok::Eof) {
            if self.check(&Tok::Spread) {
                self.advance();
                // Spread manbasi atom (ident yoki qavsli ifoda) — keyingi `[k]:v`
                // ni indeks deb yutmasligi uchun postfix EMAS, primary ishlatamiz.
                let e = self.parse_primary()?;
                entries.push(MapEntry::Spread(e));
            } else if self.check(&Tok::LBracket) {
                // dinamik kalit: [k]:v
                self.advance();
                let k = self.parse_expr()?;
                self.expect(&Tok::RBracket, "']'")?;
                self.expect(&Tok::Colon, "':'")?;
                let v = self.parse_expr()?;
                entries.push(MapEntry::Dynamic { key: k, value: v });
            } else {
                // kalit: ident yoki string-literal
                let key = match self.peek().clone() {
                    Tok::Ident(s) => {
                        self.advance();
                        s
                    }
                    Tok::Str(parts) => {
                        self.advance();
                        // faqat oddiy literal string kalit sifatida
                        if let [StrPart::Lit(s)] = parts.as_slice() {
                            s.clone()
                        } else {
                            return Err(format!(
                                "{}-qatorda map kaliti oddiy matn bo'lishi kerak",
                                self.line()
                            ));
                        }
                    }
                    other => {
                        return Err(format!(
                            "{}-qatorda map kaliti kutilgan, {:?} topildi",
                            self.line(),
                            other
                        ));
                    }
                };
                self.expect(&Tok::Colon, "':'")?;
                let value = self.parse_expr()?;
                entries.push(MapEntry::Pair { key, value });
            }
            self.eat(&Tok::Comma);
            self.skip_newlines();
        }
        self.no_app = saved;
        self.expect(&Tok::RBrace, "'}'")?;
        Ok(Expr::Map(entries))
    }

    fn parse_lambda(&mut self) -> ParseResult<Expr> {
        self.advance(); // backslash
        let mut params = Vec::new();
        while let Tok::Ident(_) = self.peek() {
            let p = self.expect_ident("lambda parametri")?;
            if params.contains(&p) {
                return Err(format!("lambda'da takror parametr nomi: '{}'", p));
            }
            params.push(p);
        }
        self.expect(&Tok::Arrow, "'->'")?;
        let body = self.parse_arrow_body()?;
        Ok(Expr::Lambda { params, body })
    }

    fn parse_if(&mut self) -> ParseResult<Expr> {
        self.advance(); // if
        let mut arms = Vec::new();
        let cond = self.parse_expr()?;
        self.expect(&Tok::Newline, "if tanasi")?;
        let block = self.parse_block()?;
        arms.push((cond, block));
        let mut else_block = None;
        loop {
            self.skip_newlines();
            if self.check(&Tok::Elif) {
                self.advance();
                let c = self.parse_expr()?;
                self.expect(&Tok::Newline, "elif tanasi")?;
                let b = self.parse_block()?;
                arms.push((c, b));
            } else if self.check(&Tok::Else) {
                self.advance();
                self.expect(&Tok::Newline, "else tanasi")?;
                else_block = Some(self.parse_block()?);
                break;
            } else {
                break;
            }
        }
        Ok(Expr::If(Box::new(IfExpr { arms, else_block })))
    }

    fn parse_match(&mut self) -> ParseResult<Expr> {
        self.advance(); // match
        let subject = self.parse_expr()?;
        self.expect(&Tok::Newline, "match tanasi")?;
        self.expect(&Tok::Indent, "match armlari (chekinish)")?;
        let mut arms = Vec::new();
        self.skip_newlines();
        while !self.check(&Tok::Dedent) && !self.check(&Tok::Eof) {
            let pattern = match self.peek().clone() {
                Tok::Sym(s) => {
                    self.advance();
                    MatchPat::Sym(s)
                }
                Tok::Int(n) => {
                    self.advance();
                    MatchPat::Int(n)
                }
                Tok::Ident(id) if id == "_" => {
                    self.advance();
                    MatchPat::Wildcard
                }
                other => {
                    return Err(format!(
                        "{}-qatorda match patterni (symbol/son/_) kutilgan, {:?} topildi",
                        self.line(),
                        other
                    ));
                }
            };
            self.expect(&Tok::Arrow, "'->'")?;
            let body = self.parse_arrow_body()?;
            arms.push(MatchArm { pattern, body });
            self.skip_newlines();
        }
        self.expect(&Tok::Dedent, "match oxiri")?;
        Ok(Expr::Match(Box::new(MatchExpr { subject, arms })))
    }

    // --- yordamchi predikatlar ---
    fn expect_ident(&mut self, what: &str) -> ParseResult<String> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.advance();
                Ok(s)
            }
            other => Err(format!(
                "{}-qatorda {} kutilgan, {:?} topildi",
                self.line(),
                what,
                other
            )),
        }
    }

    // Atom boshlanishi mumkinmi? (qavssiz chaqiruvda argument chegarasini
    // aniqlash uchun). Operatorlar, blok-chegaralar va kalit so'zlar atom
    // boshlamaydi.
    fn is_atom_start(&self) -> bool {
        matches!(
            self.peek(),
            Tok::Int(_)
                | Tok::Flt(_)
                | Tok::Str(_)
                | Tok::Sym(_)
                | Tok::Ident(_)
                | Tok::True
                | Tok::False
                | Tok::Nil
                | Tok::LParen
                | Tok::LBracket
                | Tok::LBrace
                | Tok::Backslash
        )
    }
}
