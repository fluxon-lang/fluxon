// Some AST fields (use/tbl details) are not yet read by the core interpreter —
// they are used in the batteries (db/http) phase. For that reason we silence
// dead-code warnings in this module.
#![allow(dead_code)]

// Fluxon AST — the tree the parser builds.
//
// In Fluxon the statement/expression distinction is soft: most things are
// expressions (if/match return a value), but a few are statements only (bind,
// fn declaration, each). So we keep both notions.

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Flt(f64),
    Bool(bool),
    Nil,
    Sym(String),
    // String interpolation: a mix of text pieces and expression pieces.
    Str(Vec<StrPiece>),
    Ident(String),
    List(Vec<Expr>),
    // Map: key (string) -> value. We also support spread pieces.
    Map(Vec<MapEntry>),

    // Operators
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },

    // a.b  or  a.0 (member/index via dot)
    Field {
        target: Box<Expr>,
        name: String,
    },
    // a[k] (dynamic index)
    Index {
        target: Box<Expr>,
        key: Box<Expr>,
    },

    // Parenthesis-free call: callee arg1 arg2 ...
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },

    // \a b -> body   (lambda). The body may be one expression or a block.
    Lambda {
        params: Vec<String>,
        body: Vec<Stmt>,
    },

    // error-propagate: expr!
    Try(Box<Expr>),

    // try/catch — catches an error (issue #125). `body` runs; if a `fail` or a
    // runtime error is raised, `catch_body` runs and `catch_var` (if present) is
    // bound to the error map ({message, status}). As a value it returns the last
    // expression of body on success, of catch_body on error.
    TryCatch {
        body: Vec<Stmt>,
        catch_var: Option<String>,
        catch_body: Vec<Stmt>,
    },

    // fail [status] msg — also as an expression (e.g. `x ?? (fail "...")`).
    // Never returns a value; it breaks the flow.
    Fail {
        status: Option<Box<Expr>>,
        message: Box<Expr>,
    },

    // if/elif/else — as an expression (returns a value)
    If(Box<IfExpr>),
    // match — expression
    Match(Box<MatchExpr>),
    // 1..5 range
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
    },
    // inf — infinite iterator. Only meaningful in `each i in inf` (i = 0,1,2,...).
    Inf,
}

#[derive(Debug, Clone)]
pub enum StrPiece {
    Lit(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum MapEntry {
    Pair { key: String, value: Expr },
    // {...m}  -> spread another map
    Spread(Expr),
    // {[k]:v} -> dynamic (computed) key
    Dynamic { key: Expr, value: Expr },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg, // -x
    Not, // !x
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Coalesce, // ??
    Pipe,     // |>
}

#[derive(Debug, Clone)]
pub struct IfExpr {
    // (condition, block) pairs: the first is `if`, the rest are `elif`.
    pub arms: Vec<(Expr, Vec<Stmt>)>,
    pub else_block: Option<Vec<Stmt>>,
}

#[derive(Debug, Clone)]
pub struct MatchExpr {
    pub subject: Expr,
    pub arms: Vec<MatchArm>,
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: MatchPat,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum MatchPat {
    Sym(String),
    Int(i64),
    Wildcard, // _
}

#[derive(Debug, Clone)]
pub enum Stmt {
    // x = expr   (bind a local — Python-style; re-binding is allowed)
    Bind {
        name: String,
        value: Expr,
    },
    // target <- expr  (reassign, reaching out to an enclosing/global var). target
    // may be a plain ident (`x <- v`) or a member field (`req.ctx <- v`, issue
    // #68). A field-target is only used to write into a shared ctx cell like
    // `req.ctx` (interp::assign_field) — a plain Map value is not mutated in place.
    Assign {
        target: Box<Expr>,
        value: Expr,
    },

    // fn name params... -> body
    FnDecl {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
        exported: bool,
    },

    // each x in iter / each k, v in iter
    Each {
        vars: Vec<String>,
        iter: Expr,
        body: Vec<Stmt>,
    },

    Ret(Option<Expr>),
    Skip,
    Stop,
    // fail [status] "message"
    Fail {
        status: Option<Expr>,
        message: Expr,
    },

    // use http db   /   use ./tools as t
    Use {
        items: Vec<UseItem>,
    },

    // tbl name ... (schema declaration; ignored by the core version, but still
    // parsed — used when the DB battery arrives)
    Tbl {
        name: String,
        columns: Vec<TblColumn>,
        // index/uniq declarations: both single-column ones promoted from a column
        // (`status sym index`) and multi-column parenthesized rows (`uniq(a b)`)
        // land here. Auto-migration computes CREATE/DROP INDEX from this list.
        indexes: Vec<TblIndex>,
    },

    // exp NAME = expr  (exported value)
    ExpBind {
        name: String,
        value: Expr,
    },

    // A bare expression as a statement (call, if-as-statement, ...)
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct UseItem {
    pub path: String, // "http" or "./tools"
    pub alias: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TblColumn {
    pub name: String,
    pub type_name: String,
    pub modifiers: Vec<String>,
}

// An index/unique declaration inside a tbl. `columns` is one or more columns
// (`index(a b)` -> two). `unique` is true for `uniq`/`uniq(...)`.
#[derive(Debug, Clone)]
pub struct TblIndex {
    pub columns: Vec<String>,
    pub unique: bool,
}

pub type Program = Vec<Stmt>;
