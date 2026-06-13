// Fluxon parser — builds an AST from tokens.
//
// Two layers:
//   1) Statement/block level (recursive descent): relies on Indent/Dedent/Newline
//      tokens.
//   2) Expression level (precedence climbing): operator precedence.
//
// The trickiest part is PARENTHESIS-FREE CALLS. In Fluxon `f a b` = a call,
// `a + b` = an operator. The solution: at the top ("application") level we
// collect consecutive "atoms"; if more than one atom sits side by side, the
// first is the callee and the rest are arguments. An atom is the smallest
// complete expression read until an operator or a block-boundary token is hit.

use crate::ast::*;
use crate::token::{StrPart, Tok, Token};

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    // Inside a list/map literal parenthesis-free (juxtaposition) calls are not
    // used — there each element is an atom or a parenthesized call. This flag
    // disables the application stage in that context, so in `{a:f b:g}` `f` does
    // not swallow `b` as an argument. For a call use: `{a:(f x)}`.
    no_app: bool,
    // Recursive-descent depth (nested expressions/blocks). Unbounded deep nesting
    // (~2000 parens) would fill the native stack and ABORT the process — the
    // limit returns a clean parse error before that (issue #90).
    depth: usize,
}

// Maximum depth for nested expressions/blocks. Real code does not exceed a few
// dozen levels; 256 leaves a safe margin. The native stack grows in segments via
// `stacker::maybe_grow` (as in interp), so the real limit is this counter — even
// on a 2MB thread we get a clean parse error rather than an abort.
const MAX_NEST_DEPTH: usize = 256;

// stacker parameters: the red zone is larger than the native stack used by one
// nesting level (the parse_expr -> parse_binary -> ... -> parse_primary chain);
// the segment size fits a few hundred levels.
const STACK_RED_ZONE: usize = 64 * 1024;
const STACK_GROW_SIZE: usize = 1024 * 1024;

pub type ParseResult<T> = Result<T, String>;

pub fn parse(toks: Vec<Token>) -> ParseResult<Program> {
    let mut p = Parser {
        toks,
        pos: 0,
        no_app: false,
        depth: 0,
    };
    p.parse_program()
}

impl Parser {
    // --- token stream helpers ---
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
    // Whether whitespace precedes the current token (for grammatical separation).
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
                "expected {} on line {}, but found {:?}",
                what,
                self.line(),
                self.peek()
            ))
        }
    }
    // Consumes the newlines at a statement boundary.
    fn skip_newlines(&mut self) {
        while self.check(&Tok::Newline) {
            self.advance();
        }
    }

    // --- program ---
    fn parse_program(&mut self) -> ParseResult<Program> {
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.check(&Tok::Eof) {
            stmts.push(self.parse_stmt()?);
            self.skip_newlines();
        }
        Ok(stmts)
    }

    // Increments the depth counter; a clean parse error if the limit is exceeded.
    // The caller must do `self.depth -= 1` regardless of success/error
    // (parse_expr/parse_block wrap it that way).
    fn enter_depth(&mut self) -> ParseResult<()> {
        if self.depth >= MAX_NEST_DEPTH {
            return Err(format!(
                "expression/block nested too deep on line {} (exceeded {} levels) — simplify it",
                self.line(),
                MAX_NEST_DEPTH
            ));
        }
        self.depth += 1;
        Ok(())
    }

    // A block wrapped in Indent...Dedent. The caller first consumes the Newline,
    // ensuring an Indent follows.
    fn parse_block(&mut self) -> ParseResult<Vec<Stmt>> {
        self.enter_depth()?;
        let r = stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW_SIZE, || self.parse_block_inner());
        self.depth -= 1;
        r
    }

    fn parse_block_inner(&mut self) -> ParseResult<Vec<Stmt>> {
        self.expect(&Tok::Indent, "block (indentation)")?;
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.check(&Tok::Dedent) && !self.check(&Tok::Eof) {
            stmts.push(self.parse_stmt()?);
            self.skip_newlines();
        }
        self.expect(&Tok::Dedent, "end of block")?;
        Ok(stmts)
    }

    // The body after `->`: a single-line expression OR a block on a new line.
    fn parse_arrow_body(&mut self) -> ParseResult<Vec<Stmt>> {
        if self.check(&Tok::Newline) {
            self.advance();
            self.parse_block()
        } else {
            // single-line: one expression
            let e = self.parse_expr()?;
            Ok(vec![Stmt::Expr(e)])
        }
    }

    // --- statements ---
    fn parse_stmt(&mut self) -> ParseResult<Stmt> {
        match self.peek() {
            Tok::Fn => self.parse_fn(false),
            Tok::Exp => self.parse_exp(),
            Tok::If => Ok(Stmt::Expr(self.parse_if()?)),
            Tok::Match => Ok(Stmt::Expr(self.parse_match()?)),
            Tok::Each => self.parse_each(),
            Tok::Try => Ok(Stmt::Expr(self.parse_try()?)),
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
                // any other expression statement
                let e = self.parse_expr()?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    // Started by an ident: a bind (=), an assign (<-), or a call expression.
    fn parse_ident_stmt(&mut self) -> ParseResult<Stmt> {
        // `name = ...` — a bind is only allowed on a plain ident (spec: `=` is an
        // immutable new name). We detect this up front via `peek2` so that `name`
        // is not mistaken for a call argument (`f name`).
        if let Tok::Ident(name) = self.peek().clone()
            && matches!(self.peek2(), Tok::Eq)
        {
            self.advance(); // name
            self.advance(); // =
            let value = self.parse_expr()?;
            return Ok(Stmt::Bind { name, value });
        }
        // Otherwise we read the left side as an expression. If `<-` follows, this
        // is an assign (`x <- v` or `req.ctx <- v`); otherwise a plain expression
        // statement. `<-` is a statement-level token (not an operator), so
        // parse_expr stops before it — the left expression is read in full.
        let lhs = self.parse_expr()?;
        if self.eat(&Tok::Assign) {
            let value = self.parse_expr()?;
            return Ok(Stmt::Assign {
                target: Box::new(lhs),
                value,
            });
        }
        Ok(Stmt::Expr(lhs))
    }

    fn parse_fn(&mut self, exported: bool) -> ParseResult<Stmt> {
        self.advance(); // fn
        let name = self.expect_ident("function name")?;
        let mut params = Vec::new();
        while let Tok::Ident(_) = self.peek() {
            let p = self.expect_ident("parameter")?;
            if params.contains(&p) {
                return Err(format!(
                    "duplicate parameter name in function '{}': '{}'",
                    name, p
                ));
            }
            params.push(p);
        }
        if self.eat(&Tok::Arrow) {
            // single-line: fn double x -> x * 2
            let body = self.parse_arrow_body()?;
            Ok(Stmt::FnDecl {
                name,
                params,
                body,
                exported,
            })
        } else {
            self.expect(&Tok::Newline, "function body")?;
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
        let name = self.expect_ident("export name")?;
        self.expect(&Tok::Eq, "'='")?;
        let value = self.parse_expr()?;
        Ok(Stmt::ExpBind { name, value })
    }

    fn parse_each(&mut self) -> ParseResult<Stmt> {
        self.advance(); // each
        let mut vars = vec![self.expect_ident("loop variable")?];
        if self.eat(&Tok::Comma) {
            vars.push(self.expect_ident("second loop variable")?);
        }
        self.expect(&Tok::In, "'in'")?;
        let iter = self.parse_expr()?;
        self.expect(&Tok::Newline, "each body")?;
        let body = self.parse_block()?;
        Ok(Stmt::Each { vars, iter, body })
    }

    fn parse_fail(&mut self) -> ParseResult<Stmt> {
        let e = self.parse_fail_expr()?;
        Ok(Stmt::Expr(e))
    }

    // `fail [status] message` — as an expression (the statement uses this too).
    // The arguments after `fail` are collected like a parenthesis-free application.
    fn parse_fail_expr(&mut self) -> ParseResult<Expr> {
        self.advance(); // fail
        let first = self.parse_postfix()?;
        // fail 422 "msg"  -> status + message ;  fail "msg" -> message only
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
                // ./tools  ->  Slash? how does the lexer actually emit './tools'?
                // './tools' = Dot Slash Ident. We collect that.
                // ../lib/x too ('..' is the parent dir) — parse_module_path collects both.
                Tok::Dot | Tok::DotDot => self.parse_module_path()?,
                _ => break,
            };
            let alias = if self.eat(&Tok::As) {
                Some(self.expect_ident("alias name")?)
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

    // Collects a module path like ./tools  or  ../lib/x.
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
        let name = self.expect_ident("table name")?;
        self.expect(&Tok::Newline, "table body")?;
        self.expect(&Tok::Indent, "table columns (indentation)")?;
        let mut columns = Vec::new();
        let mut indexes: Vec<TblIndex> = Vec::new();
        self.skip_newlines();
        while !self.check(&Tok::Dedent) && !self.check(&Tok::Eof) {
            // Multi-column index/uniq row:  index(a b)  /  uniq(a, b).
            // Distinguished from a column row by `index`/`uniq` immediately
            // followed by `(`. In a plain column the 2nd token is a type-ident or
            // a Newline, never `(` — so this is safe. (Inside parens the lexer
            // emits no Newline, so a multi-line `uniq(\n a\n b\n)` works too.)
            if let Tok::Ident(kw) = self.peek().clone()
                && (kw == "index" || kw == "uniq")
                && *self.peek2() == Tok::LParen
            {
                self.advance(); // index|uniq
                self.advance(); // (
                let mut cols = Vec::new();
                while !self.check(&Tok::RParen) && !self.check(&Tok::Eof) {
                    // Comma is optional (separation is whitespace by default);
                    // also accepted for an agent who mistakenly wrote `index(a, b)`.
                    if self.eat(&Tok::Comma) {
                        continue;
                    }
                    cols.push(self.expect_ident("index column")?);
                }
                self.expect(&Tok::RParen, "index parenthesis")?;
                indexes.push(TblIndex {
                    columns: cols,
                    unique: kw == "uniq",
                });
                self.skip_newlines();
                continue;
            }

            // column:  name type mod1 mod2...  (modifiers separated by whitespace OR `|`)
            let col_name = self.expect_ident("column name")?;
            let mut modifiers = Vec::new();
            let mut type_name = String::new();
            if let Tok::Ident(_) = self.peek() {
                type_name = self.expect_ident("column type")?;
            }
            // Modifier loop: ident -> push; if a `|` follows, consume it and
            // continue (`index|uniq`). The spaced form (`index uniq`) uses this
            // loop too.
            loop {
                if let Tok::Ident(m) = self.peek().clone() {
                    self.advance();
                    // `ref:tbl.col` — an FK modifier. If `:` immediately follows
                    // `ref`, take a special branch: read the target `tbl.col` and
                    // store it as a single `ref:tbl.col` modifier string (db_mod
                    // turns it into FOREIGN KEY ... REFERENCES).
                    if m == "ref" && self.check(&Tok::Colon) {
                        self.advance(); // :
                        let target_tbl = self.expect_ident("ref table name")?;
                        self.expect(&Tok::Dot, "ref `tbl.col`")?;
                        let target_col = self.expect_ident("ref column name")?;
                        modifiers.push(format!("ref:{target_tbl}.{target_col}"));
                    } else {
                        modifiers.push(m);
                    }
                } else if self.check(&Tok::Pipe) {
                    self.advance();
                } else {
                    break;
                }
            }
            // Promote a single-column index/uniq modifier into a TblIndex.
            // `uniq` subsumes `index` — one unique index (not two).
            let has = |m: &str| modifiers.iter().any(|x| x == m);
            if has("index") || has("uniq") {
                indexes.push(TblIndex {
                    columns: vec![col_name.clone()],
                    unique: has("uniq"),
                });
            }
            // Silently skip unrecognized leftover tokens (future modifiers) to
            // the end of the line — `ref:` is already handled above.
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
        self.expect(&Tok::Dedent, "end of table")?;
        Ok(Stmt::Tbl {
            name,
            columns,
            indexes,
        })
    }

    // --- expressions (precedence climbing) ---
    // Every nested expression (parens, list/map element, index, interpolation)
    // passes through here — the single control point for the depth limit.
    fn parse_expr(&mut self) -> ParseResult<Expr> {
        self.enter_depth()?;
        let r = stacker::maybe_grow(STACK_RED_ZONE, STACK_GROW_SIZE, || self.parse_binary(0));
        self.depth -= 1;
        r
    }

    // Range `..` precedence: LOWER than arithmetic, but HIGHER than
    // pipe/comparison/logic. So `1..n+1` = `1..(n+1)` (arithmetic binds inside the
    // endpoint), `1..3 |> f` = `(1..3) |> f` (the pipe applies to the whole range).
    const RANGE_PREC: u8 = 7;

    // Operator precedence table. Smaller number = lower precedence.
    // `..` (RANGE_PREC = 7) sits between arithmetic and pipe.
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
            Tok::Plus => (BinOp::Add, 8),
            Tok::Minus => (BinOp::Sub, 8),
            Tok::Star => (BinOp::Mul, 9),
            Tok::Slash => (BinOp::Div, 9),
            Tok::Percent => (BinOp::Mod, 9),
            _ => return None,
        })
    }

    fn parse_binary(&mut self, min_prec: u8) -> ParseResult<Expr> {
        let mut lhs = self.parse_application()?;
        loop {
            // `..` is woven into the precedence ladder (Range is not a BinOp, hence
            // a separate branch). Arithmetic binds on the right side
            // (RANGE_PREC + 1), and lower operators (pipe etc.) wrap the range.
            if self.check(&Tok::DotDot) {
                if Self::RANGE_PREC < min_prec {
                    break;
                }
                self.advance();
                let rhs = self.parse_binary(Self::RANGE_PREC + 1)?;
                lhs = Expr::Range {
                    start: Box::new(lhs),
                    end: Box::new(rhs),
                };
                continue;
            }
            let Some((op, prec)) = Self::bin_prec(self.peek()) else {
                break;
            };
            if prec < min_prec {
                break;
            }
            self.advance();
            // left-associative: the right side with a higher precedence
            let rhs = self.parse_binary(prec + 1)?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    // Parenless call: atom atom atom...
    fn parse_application(&mut self) -> ParseResult<Expr> {
        let first = self.parse_postfix()?;
        // Inside a list/map literal juxtaposition-call is disabled.
        if self.no_app {
            return Ok(first);
        }
        // `cron.on` SPECIAL: the first argument is a standard Unix 5-field cron
        // expression, written WITHOUT QUOTES (`cron.on 0 * * * * f`). `*` here is
        // NOT multiplication — it is a cron token. The quoted variant
        // (`cron.on "0 * * * *" f`) has no special mode and passes through as a
        // plain str (the condition below does not fire on `Str`).
        if is_cron_on(&first) && self.is_cron_field_start() {
            return self.parse_cron_application(first);
        }
        // If the next token starts another atom — this is a call.
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

    // `cron.on <5 fields> <handler...>` — collects the cron expression into a str
    // and reads the rest of the arguments as usual. Called ONLY when the callee is
    // exactly `cron.on`, so it does not affect other calls.
    fn parse_cron_application(&mut self, callee: Expr) -> ParseResult<Expr> {
        let expr = self.parse_cron_fields()?;
        let mut args = vec![Expr::Str(vec![StrPiece::Lit(expr)])];
        // The remaining arguments (a named function or lambda) — ordinary juxtaposition.
        while self.is_atom_start() {
            args.push(self.parse_postfix()?);
        }
        Ok(Expr::Call {
            callee: Box::new(callee),
            args,
        })
    }

    // Reads the cron 5-field sequence (`0 */15 1,2,3 * 1-5`) from the token stream
    // and collects it into a single str. Cron tokens: Int/Star/Slash/Minus/Comma.
    // If a token has `spaced` before it we insert a space (the field separator).
    // We stop at the first NON-cron token (Ident/Backslash/Newline...) — that is
    // the handler argument or the end of the line.
    fn parse_cron_fields(&mut self) -> ParseResult<String> {
        let mut out = String::new();
        let mut first = true;
        while self.is_cron_field_token() {
            if !first && self.spaced() {
                out.push(' ');
            }
            first = false;
            match self.peek().clone() {
                Tok::Int(n) => out.push_str(&n.to_string()),
                Tok::Star => out.push('*'),
                Tok::Slash => out.push('/'),
                Tok::Minus => out.push('-'),
                Tok::Comma => out.push(','),
                _ => unreachable!("is_cron_field_token guarantees this"),
            }
            self.advance();
        }
        if out.is_empty() {
            return Err(format!(
                "expected a cron expression after cron.on on line {}",
                self.line()
            ));
        }
        Ok(out)
    }

    // Does the current token start a cron field (for the 5-field collection)?
    // Int or Star starts a field (Slash/Minus/Comma only occur inside a field).
    fn is_cron_field_start(&self) -> bool {
        matches!(self.peek(), Tok::Int(_) | Tok::Star)
    }

    // Is the current token part of a cron expression?
    fn is_cron_field_token(&self) -> bool {
        matches!(
            self.peek(),
            Tok::Int(_) | Tok::Star | Tok::Slash | Tok::Minus | Tok::Comma
        )
    }

    // postfix: .field, [index], ! (try)
    fn parse_postfix(&mut self) -> ParseResult<Expr> {
        let mut e = self.parse_primary()?;
        loop {
            match self.peek() {
                Tok::Dot => {
                    self.advance();
                    // .name  or  .0 (numeric index)  or  .(expr) (computed index)
                    match self.peek().clone() {
                        Tok::Int(n) => {
                            self.advance();
                            e = Expr::Index {
                                target: Box::new(e),
                                key: Box::new(Expr::Int(n)),
                            };
                        }
                        // `.(expr)` — computed index: `xs.(i)`, `xs.(xs.len - 1)`.
                        // Builds the same Expr::Index as the bracket form (`xs[i]`);
                        // both forms are supported. Inside the parens full
                        // application is re-enabled (regardless of the `no_app` context).
                        Tok::LParen => {
                            self.advance(); // (
                            let saved = self.no_app;
                            self.no_app = false;
                            let key = self.parse_expr();
                            self.no_app = saved;
                            let key = key?;
                            self.expect(&Tok::RParen, "')'")?;
                            e = Expr::Index {
                                target: Box::new(e),
                                key: Box::new(key),
                            };
                        }
                        // Field name: a plain ident or a KEYWORD (`time.in`, `x.match`).
                        // Keywords act as names in member position — this is the
                        // Fluxon philosophy (the language adapts to AI): the AI writes
                        // a natural `time.in`, and `in`, though a global keyword, can
                        // still be a field.
                        tok => match keyword_as_name(&tok) {
                            Some(name) => {
                                self.advance();
                                e = Expr::Field {
                                    target: Box::new(e),
                                    name,
                                };
                            }
                            None => {
                                return Err(format!(
                                    "expected a name or index after '.' on line {}, found {:?}",
                                    self.line(),
                                    tok
                                ));
                            }
                        },
                    }
                }
                // Adjacent `()` — an argument-less (nullary) call (`new_id()`).
                // A parenless call is identified by its arguments, so this is the
                // only way to call a 0-arity function. `f` (parenless) is the
                // function VALUE, while `f()` is a CALL — the two meanings are
                // cleanly separated. Only EMPTY parens: not `f(x)` (the canonical
                // form is `f x`). Not the spaced `f ()` either — that is read as an
                // argument in parse_application.
                Tok::LParen if !self.spaced() => {
                    self.advance();
                    if !self.check(&Tok::RParen) {
                        return Err(format!(
                            "`f()` on line {} is only for argument-less calls; \
                             calling with arguments is written without parentheses (`f x`)",
                            self.line()
                        ));
                    }
                    self.advance(); // )
                    e = Expr::Call {
                        callee: Box::new(e),
                        args: Vec::new(),
                    };
                }
                // `[` is a postfix index ONLY when adjacent (`arr[i]`). When it
                // comes with a space (`f "x" [a]`) it is a separate list argument —
                // parse_application takes it itself.
                Tok::LBracket if !self.spaced() => {
                    self.advance();
                    let key = self.parse_expr()?;
                    self.expect(&Tok::RBracket, "']'")?;
                    e = Expr::Index {
                        target: Box::new(e),
                        key: Box::new(key),
                    };
                }
                // `!` is a postfix Try ONLY when adjacent (`db.one ...!`). When it
                // comes with a space (`log !x`) it is the start of a prefix not —
                // parse_application takes it as an argument itself.
                Tok::Bang if !self.spaced() => {
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
            Tok::Inf => {
                self.advance();
                Ok(Expr::Inf)
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
                // Inside parens full application is re-enabled.
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
            Tok::Try => self.parse_try(),
            Tok::Fail => self.parse_fail_expr(),
            other => Err(format!(
                "expected an expression on line {}, found {:?}",
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
                StrPart::Expr(src, line) => {
                    // Tokenize + parse the expression source independently.
                    // We pass the original line number to the sub-lexer — otherwise
                    // an error always misleadingly says "on line 1" (issue #106).
                    let toks = crate::lexer::lex_at(&src, line)
                        .map_err(|e| format!("inside interpolation: {}", e))?;
                    let mut sub = Parser {
                        toks,
                        pos: 0,
                        no_app: false,
                        // Inherit the outer depth — so the limit cannot be bypassed
                        // through interpolation.
                        depth: self.depth,
                    };
                    sub.skip_newlines();
                    let e = sub
                        .parse_expr()
                        .map_err(|e| format!("inside interpolation: {}", e))?;
                    pieces.push(StrPiece::Expr(e));
                }
            }
        }
        Ok(Expr::Str(pieces))
    }

    fn parse_list(&mut self) -> ParseResult<Expr> {
        self.advance(); // [
        let saved = self.no_app;
        self.no_app = true; // elements are separated by whitespace (no juxtaposition-call)
        let mut items = Vec::new();
        self.skip_newlines();
        while !self.check(&Tok::RBracket) && !self.check(&Tok::Eof) {
            // In element position too, a bare type name (`[str]`) becomes a sym —
            // consistent with map value position (`{k:str}`), for schemas.
            items.push(schema_type_sym(self.parse_expr()?));
            self.eat(&Tok::Comma); // comma optional / tolerant
            self.skip_newlines();
        }
        self.no_app = saved;
        self.expect(&Tok::RBracket, "']'")?;
        Ok(Expr::List(items))
    }

    fn parse_map(&mut self) -> ParseResult<Expr> {
        self.advance(); // {
        let saved = self.no_app;
        self.no_app = true; // values are atom/parenthesized; in `{a:f b:g}` f does not swallow g
        let mut entries = Vec::new();
        self.skip_newlines();
        while !self.check(&Tok::RBrace) && !self.check(&Tok::Eof) {
            if self.check(&Tok::Spread) {
                self.advance();
                // The spread source is an atom (ident or parenthesized expression) —
                // we use primary, NOT postfix, so it does not swallow a following
                // `[k]:v` as an index.
                let e = self.parse_primary()?;
                entries.push(MapEntry::Spread(e));
            } else if self.check(&Tok::LBracket) {
                // dynamic key: [k]:v
                self.advance();
                let k = self.parse_expr()?;
                self.expect(&Tok::RBracket, "']'")?;
                self.expect(&Tok::Colon, "':'")?;
                let v = self.parse_expr()?;
                entries.push(MapEntry::Dynamic { key: k, value: v });
            } else {
                // key: ident, keyword (`{in: 1}`) or string-literal.
                // A keyword acts as a name in a map key too — symmetric with field
                // access (`m.in`), matching the Fluxon philosophy.
                let key = match self.peek().clone() {
                    Tok::Str(parts) => {
                        self.advance();
                        // only a plain literal string as a key
                        if let [StrPart::Lit(s)] = parts.as_slice() {
                            s.clone()
                        } else {
                            return Err(format!(
                                "map key must be plain text on line {}",
                                self.line()
                            ));
                        }
                    }
                    other => match keyword_as_name(&other) {
                        Some(name) => {
                            self.advance();
                            name
                        }
                        None => {
                            return Err(format!(
                                "expected a map key on line {}, found {:?}",
                                self.line(),
                                other
                            ));
                        }
                    },
                };
                self.expect(&Tok::Colon, "':'")?;
                let value = schema_type_sym(self.parse_expr()?);
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
            let p = self.expect_ident("lambda parameter")?;
            if params.contains(&p) {
                return Err(format!("duplicate parameter name in lambda: '{}'", p));
            }
            params.push(p);
        }
        self.expect(&Tok::Arrow, "'->'")?;
        let body = self.parse_arrow_body()?;
        Ok(Expr::Lambda { params, body })
    }

    fn parse_if(&mut self) -> ParseResult<Expr> {
        self.advance(); // if
        // `if cond a else b` — the inline (ternary) form. If there is an `else` on
        // this logical line (outside parens), we read the expression form, not the
        // block form.
        if self.if_is_inline() {
            return self.parse_inline_if();
        }
        let mut arms = Vec::new();
        let cond = self.parse_expr()?;
        self.expect(&Tok::Newline, "if body")?;
        let block = self.parse_block()?;
        arms.push((cond, block));
        let mut else_block = None;
        loop {
            self.skip_newlines();
            if self.check(&Tok::Elif) {
                self.advance();
                let c = self.parse_expr()?;
                self.expect(&Tok::Newline, "elif body")?;
                let b = self.parse_block()?;
                arms.push((c, b));
            } else if self.check(&Tok::Else) {
                self.advance();
                self.expect(&Tok::Newline, "else body")?;
                else_block = Some(self.parse_block()?);
                break;
            } else {
                break;
            }
        }
        Ok(Expr::If(Box::new(IfExpr { arms, else_block })))
    }

    // If an `else` appears after `if` on this logical line (not inside parens),
    // this is the inline expression form. In the block form a Newline comes first
    // after the condition, so if we reach a depth-0 Newline first — it is a block.
    // An `else` inside parens/list/map (for example a nested inline if) is skipped
    // by tracking depth.
    fn if_is_inline(&self) -> bool {
        let mut depth = 0i32;
        let mut i = self.pos;
        while i < self.toks.len() {
            match &self.toks[i].tok {
                Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
                Tok::RParen | Tok::RBracket | Tok::RBrace => depth -= 1,
                Tok::Else if depth == 0 => return true,
                Tok::Newline | Tok::Indent | Tok::Dedent | Tok::Eof if depth <= 0 => {
                    return false;
                }
                _ => {}
            }
            i += 1;
        }
        false
    }

    // Inline if: `if cond a else b` — returns one value (the ternary equivalent).
    // Parenless (juxtaposition) calls are disabled in the condition, so the
    // condition does not swallow the `a` branch as an argument. If you need a call
    // in the condition, wrap it in parens: `if (str.empty s) "" else s`. The
    // branches are full expressions (a call is allowed too). We build it as an
    // IfExpr, so the interpreter does not change.
    fn parse_inline_if(&mut self) -> ParseResult<Expr> {
        let saved = self.no_app;
        self.no_app = true;
        let cond = self.parse_expr();
        self.no_app = saved;
        let cond = cond?;
        let then_expr = self.parse_expr()?;
        self.expect(&Tok::Else, "inline if 'else'")?;
        let else_expr = self.parse_expr()?;
        Ok(Expr::If(Box::new(IfExpr {
            arms: vec![(cond, vec![Stmt::Expr(then_expr)])],
            else_block: Some(vec![Stmt::Expr(else_expr)]),
        })))
    }

    fn parse_match(&mut self) -> ParseResult<Expr> {
        self.advance(); // match
        let subject = self.parse_expr()?;
        self.expect(&Tok::Newline, "match body")?;
        self.expect(&Tok::Indent, "match arms (indentation)")?;
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
                        "expected a match pattern (symbol/number/_) on line {}, found {:?}",
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
        self.expect(&Tok::Dedent, "end of match")?;
        Ok(Expr::Match(Box::new(MatchExpr { subject, arms })))
    }

    // try/catch — catches an error (issue #125). A block expression like
    // `if`/`match`:
    //   try
    //     <body>
    //   catch e
    //     <error handler>
    // The `catch` variable is optional (`catch` or `catch e`). Note: `catch` sits
    // at the same indentation level as `try`, like the `else` of an `if`.
    fn parse_try(&mut self) -> ParseResult<Expr> {
        self.advance(); // try
        self.expect(&Tok::Newline, "try body")?;
        let body = self.parse_block()?;
        self.skip_newlines();
        self.expect(&Tok::Catch, "'catch'")?;
        let catch_var = if let Tok::Ident(_) = self.peek() {
            Some(self.expect_ident("catch variable")?)
        } else {
            None
        };
        self.expect(&Tok::Newline, "catch body")?;
        let catch_body = self.parse_block()?;
        Ok(Expr::TryCatch {
            body,
            catch_var,
            catch_body,
        })
    }

    // --- helper predicates ---
    fn expect_ident(&mut self, what: &str) -> ParseResult<String> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.advance();
                Ok(s)
            }
            other => Err(format!(
                "expected {} on line {}, found {:?}",
                what,
                self.line(),
                other
            )),
        }
    }

    // Can an atom start here? (used to find the argument boundary in a parenless
    // call). Operators, block boundaries and keywords do not start an atom.
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
                // Prefix not (`f !x`) — can start an atom. It does not clash with
                // postfix Try: that is swallowed inside parse_postfix only when
                // adjacent (`x!`); a `!` reaching here is always after whitespace,
                // i.e. a prefix.
                | Tok::Bang
        )
    }
}

// Checks that the callee is exactly `cron.on` (Field{Ident("cron"), "on"}).
// Only this call enables the special mode of reading a cron expression unquoted.
fn is_cron_on(callee: &Expr) -> bool {
    matches!(
        callee,
        Expr::Field { target, name }
            if name == "on" && matches!(target.as_ref(), Expr::Ident(m) if m == "cron")
    )
}

// Field name after `.`: returns its textual name even if it is a keyword.
// So member names like `time.in`, `x.match`, `x.if` do not clash with keywords —
// in member position there is no grammatical meaning, only a name is needed
// (Fluxon: the language adapts to AI). Source: the inverse of the keyword table in
// the lexer's scan_ident.
// Interprets a bare type name in map value position (`{a:str}`) as a sym (equal to
// `{a::str}`). This enables the `{product:str qty:int}` syntax the docs promise for
// `ai.json`/tool schemas: `wrap_schema` already converts a sym/str type name into a
// JSON-schema type (str->string ...). Because `str` is also a module name, as a
// value it would give an "unknown name: str" error — here we convert to a sym only
// for a SINGLE, suffix-free ident; a call/field (`str.upper`) or any other
// expression is untouched.
fn schema_type_sym(value: Expr) -> Expr {
    match value {
        Expr::Ident(name) if is_schema_type_name(&name) => Expr::Sym(name),
        other => other,
    }
}

// Identifiers recognized as type names in a schema context.
// From the `tbl` column types (docs/fluxon-agent.md), the ones that map to a
// JSON-schema type.
fn is_schema_type_name(name: &str) -> bool {
    matches!(name, "str" | "int" | "flt" | "bool" | "json" | "sym")
}

fn keyword_as_name(tok: &Tok) -> Option<String> {
    let s = match tok {
        Tok::Ident(s) => return Some(s.clone()),
        Tok::Fn => "fn",
        Tok::Ret => "ret",
        Tok::If => "if",
        Tok::Elif => "elif",
        Tok::Else => "else",
        Tok::Each => "each",
        Tok::In => "in",
        Tok::Match => "match",
        Tok::Skip => "skip",
        Tok::Stop => "stop",
        Tok::Use => "use",
        Tok::Exp => "exp",
        Tok::As => "as",
        Tok::Tbl => "tbl",
        Tok::Fail => "fail",
        Tok::Try => "try",
        Tok::Catch => "catch",
        Tok::True => "true",
        Tok::False => "false",
        Tok::Nil => "nil",
        Tok::Inf => "inf",
        _ => return None,
    };
    Some(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_type_names() {
        for t in ["str", "int", "flt", "bool", "json", "sym"] {
            assert!(is_schema_type_name(t), "{} must be a type name", t);
        }
        // NON-type names are untouched (they remain variables).
        for t in ["x", "str2", "serial", "now", "money", "upper"] {
            assert!(!is_schema_type_name(t), "{} must NOT be a type name", t);
        }
    }

    #[test]
    fn schema_sym_only_for_bare_type_ident() {
        // bare type ident -> sym
        match schema_type_sym(Expr::Ident("str".to_string())) {
            Expr::Sym(s) => assert_eq!(s, "str"),
            _ => panic!("str ident must become a sym"),
        }
        // non-type ident -> unchanged
        match schema_type_sym(Expr::Ident("foo".to_string())) {
            Expr::Ident(s) => assert_eq!(s, "foo"),
            _ => panic!("foo ident must not change"),
        }
        // non-ident expression (for example an int literal) -> unchanged
        match schema_type_sym(Expr::Int(5)) {
            Expr::Int(5) => {}
            _ => panic!("Int literal must not change"),
        }
    }

    // In `s = [str int]` a bare type name in list element position becomes a sym —
    // consistent with map values, for schemas (`{blocks:[str]}`).
    fn first_expr(src: &str) -> Expr {
        let prog = parse(crate::lexer::lex(src).unwrap()).unwrap();
        match &prog[0] {
            Stmt::Bind { value, .. } => value.clone(),
            other => panic!("expected Bind, found {:?}", other),
        }
    }

    #[test]
    fn list_element_bare_type_to_sym() {
        match first_expr("s = [str int]") {
            Expr::List(items) => {
                assert!(matches!(&items[0], Expr::Sym(s) if s == "str"));
                assert!(matches!(&items[1], Expr::Sym(s) if s == "int"));
            }
            other => panic!("expected List, found {:?}", other),
        }
        // A NON-type ident stays a variable in a list.
        match first_expr("s = [x y]") {
            Expr::List(items) => {
                assert!(matches!(&items[0], Expr::Ident(s) if s == "x"));
            }
            other => panic!("expected List, found {:?}", other),
        }
    }

    // Extracts the indexes from a tbl body.
    fn tbl_indexes(src: &str) -> Vec<TblIndex> {
        let prog = parse(crate::lexer::lex(src).unwrap()).unwrap();
        match &prog[0] {
            Stmt::Tbl { indexes, .. } => indexes.clone(),
            other => panic!("expected Tbl, found {:?}", other),
        }
    }

    #[test]
    fn tbl_single_and_multi_index() {
        // `b sym index` -> single index; `uniq(a b)` -> multi unique.
        let idx = tbl_indexes("tbl t\n  a int\n  b sym index\n  uniq(a b)\n");
        assert_eq!(idx.len(), 2);
        assert_eq!(idx[0].columns, vec!["b"]);
        assert!(!idx[0].unique);
        assert_eq!(idx[1].columns, vec!["a", "b"]);
        assert!(idx[1].unique);
    }

    #[test]
    fn tbl_pipe_modifier_one_unique_index() {
        // `c sym index|uniq` -> a single unique index (uniq subsumes index).
        let idx = tbl_indexes("tbl t\n  c sym index|uniq\n");
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].columns, vec!["c"]);
        assert!(idx[0].unique);
    }

    #[test]
    fn tbl_index_comma_optional() {
        // `index(a, b)` (comma form) gives the same result as `index(a b)`.
        let comma = tbl_indexes("tbl t\n  index(a, b)\n");
        let space = tbl_indexes("tbl t\n  index(a b)\n");
        assert_eq!(comma.len(), 1);
        assert_eq!(comma[0].columns, vec!["a", "b"]);
        assert!(!comma[0].unique);
        assert_eq!(comma[0].columns, space[0].columns);
    }

    #[test]
    fn tbl_spaced_modifier_still_works() {
        // The spaced `index uniq` is also accepted (the canonical form is `|`).
        let idx = tbl_indexes("tbl t\n  d sym index uniq\n");
        assert_eq!(idx.len(), 1);
        assert!(idx[0].unique);
    }
}
