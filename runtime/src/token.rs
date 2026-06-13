// Fluxon tokens — the smallest meaningful units the lexer produces.
//
// Most important responsibility: Fluxon is indentation-sensitive, so besides
// ordinary symbols the lexer also emits synthetic `Indent`/`Dedent` tokens
// marking block start/end (just like Python). A statement boundary is marked
// by `Newline` — Fluxon has no `;`.

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    // Literals
    Int(i64),
    Flt(f64),
    Str(Vec<StrPart>), // with interpolation pieces
    Sym(String),       // :ok  -> "ok"
    Ident(String),
    True,
    False,
    Nil,
    Inf, // infinite iterator (each i in inf)

    // Keywords
    Fn,
    Ret,
    If,
    Elif,
    Else,
    Each,
    In,
    Match,
    Skip,
    Stop,
    Use,
    Exp,
    As,
    Tbl,
    Fail,
    Try,
    Catch,

    // Operators and punctuation
    Eq,        // =
    Assign,    // <-  (mutable bind / reassignment)
    Plus,      // +
    Minus,     // -
    Star,      // *
    Slash,     // /
    Percent,   // %
    EqEq,      // ==
    NotEq,     // !=
    Lt,        // <
    LtEq,      // <=
    Gt,        // >
    GtEq,      // >=
    Amp,       // &  (and)
    Pipe,      // |  (or)
    Bang,      // !  (not / error-propagate)
    Question2, // ?? (null-coalesce)
    Dot,       // .
    DotDot,    // .. (range)
    PipeGt,    // |> (pipe)
    Arrow,     // -> (lambda/fn body)
    Backslash, // \  (lambda start)
    Colon,     // :  (map key separator)
    LParen,    // (
    RParen,    // )
    LBracket,  // [  (list opener)
    RBracket,  // ]
    LBrace,    // {  (map opener)
    RBrace,    // }
    Comma,     // , (not officially part of Fluxon, but captured for error reporting)
    Spread,    // ... (map/list spread — added in round3)

    // Structure
    Newline,
    Indent,
    Dedent,
    Eof,
}

// String literal interpolation pieces: "salom ${name}!" ->
// [Lit("salom "), Expr("name", 1)]
#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    Lit(String),
    // Expression source text + the line number where it started in the source.
    // The parser re-lexes and re-parses the expression; the line number is kept
    // so error diagnostics point at the original line (rather than collapsing to
    // line 1).
    Expr(String, usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    pub col: usize,
    // Whether whitespace (or a tab/newline) preceded this token. Used by the
    // grammar to distinguish `arr[i]` (adjacent -> index) from `f "x" [a]`
    // (spaced -> a separate argument).
    pub spaced: bool,
}
