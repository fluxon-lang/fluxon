// Ba'zi AST maydonlari (use/tbl tafsilotlari) yadro interpreterda hali
// o'qilmaydi — ular batteries (db/http) fazasida ishlatiladi. Shu sababli
// dead-code ogohlantirishlarini bu modulda o'chiramiz.
#![allow(dead_code)]

// Fluxon AST — parser quradigan daraxt.
//
// Fluxon'da statement va expression farqi yumshoq: ko'p narsa expression
// (if/match qiymat qaytaradi), lekin ba'zilari faqat statement (bind, fn e'lon,
// each). Shuning uchun ikkala tushunchani ham saqlaymiz.

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Flt(f64),
    Bool(bool),
    Nil,
    Sym(String),
    // String interpolatsiya: matn bo'laklari va ifoda bo'laklari aralash.
    Str(Vec<StrPiece>),
    Ident(String),
    List(Vec<Expr>),
    // Map: kalit (string) -> qiymat. Spread bo'laklarini ham qo'llab-quvvatlaymiz.
    Map(Vec<MapEntry>),

    // Operatorlar
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },

    // a.b  yoki  a.0 (member/index nuqta orqali)
    Field {
        target: Box<Expr>,
        name: String,
    },
    // a[k] (dinamik indeks)
    Index {
        target: Box<Expr>,
        key: Box<Expr>,
    },

    // Qavssiz chaqirish: callee arg1 arg2 ...
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },

    // \a b -> tana   (lambda). Tana bir ifoda yoki blok bo'lishi mumkin.
    Lambda {
        params: Vec<String>,
        body: Vec<Stmt>,
    },

    // error-propagate: expr!
    Try(Box<Expr>),

    // try/catch — xatoni ushlab qoladi (issue #125). `body` ishga tushadi; agar
    // `fail` yoki runtime xato ko'tarilsa, `catch_body` ishlaydi va `catch_var`
    // (bo'lsa) xato map'iga ({message, status}) bog'lanadi. Qiymat sifatida
    // muvaffaqiyatda body, xatoda catch_body oxirgi ifodasini qaytaradi.
    TryCatch {
        body: Vec<Stmt>,
        catch_var: Option<String>,
        catch_body: Vec<Stmt>,
    },

    // fail [status] msg — ifoda sifatida ham (masalan `x ?? (fail "...")`).
    // Hech qachon qiymat qaytarmaydi; oqimni uzadi.
    Fail {
        status: Option<Box<Expr>>,
        message: Box<Expr>,
    },

    // if/elif/else — expression sifatida (qiymat qaytaradi)
    If(Box<IfExpr>),
    // match — expression
    Match(Box<MatchExpr>),
    // 1..5 range
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
    },
    // inf — cheksiz iterator. Faqat `each i in inf` da ma'noli (i = 0,1,2,...).
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
    // {...m}  -> boshqa mapni yoyish
    Spread(Expr),
    // {[k]:v} -> dinamik (hisoblangan) kalit
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
    // (shart, blok) juftliklari: birinchisi `if`, qolganlari `elif`.
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
    // x = expr   (immutable)
    Bind {
        name: String,
        value: Expr,
    },
    // target <- expr  (mutable bind yoki qayta tayinlash). target oddiy ident
    // (`x <- v`) yoki member field (`req.ctx <- v`, issue #68) bo'lishi mumkin.
    // Field-target faqat `req.ctx` kabi shared ctx cell'ga yozish uchun ishlatiladi
    // (interp::assign_field) — oddiy Map immutable bo'lib qoladi.
    Assign {
        target: Box<Expr>,
        value: Expr,
    },

    // fn nom params... -> body
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
    // fail [status] "xabar"
    Fail {
        status: Option<Expr>,
        message: Expr,
    },

    // use http db   /   use ./tools as t
    Use {
        items: Vec<UseItem>,
    },

    // tbl nom ... (schema e'loni; yadro versiyada e'tiborga olinmaydi, lekin
    // parse qilinadi — DB battery kelganda ishlatiladi)
    Tbl {
        name: String,
        columns: Vec<TblColumn>,
        // index/uniq e'lonlari: single-ustun (`status sym index`) promotion
        // qilingani ham, multi-ustun qavsli qator (`uniq(a b)`) ham shu yerga
        // tushadi. Auto-migration shu ro'yxatdan CREATE/DROP INDEX hisoblaydi.
        indexes: Vec<TblIndex>,
    },

    // exp NAME = expr  (eksport qilingan qiymat)
    ExpBind {
        name: String,
        value: Expr,
    },

    // Yakka ifoda statement sifatida (chaqiruv, if-as-statement, ...)
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct UseItem {
    pub path: String, // "http" yoki "./tools"
    pub alias: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TblColumn {
    pub name: String,
    pub type_name: String,
    pub modifiers: Vec<String>,
}

// tbl ichidagi index/unikal e'loni. `columns` bir yoki ko'p ustun
// (`index(a b)` → ikkita). `unique` — `uniq`/`uniq(...)` bo'lsa true.
#[derive(Debug, Clone)]
pub struct TblIndex {
    pub columns: Vec<String>,
    pub unique: bool,
}

pub type Program = Vec<Stmt>;
