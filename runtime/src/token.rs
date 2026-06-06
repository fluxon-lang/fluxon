// Flux tokenlari — lexer chiqaradigan eng kichik ma'noli birliklar.
//
// Flux indentation-sezgir til, shuning uchun lexer oddiy belgilardan tashqari
// blok boshlanishi/tugashini bildiruvchi sun'iy `Indent`/`Dedent` tokenlarini
// ham chiqaradi (xuddi Python kabi). Statement chegarasi `Newline` bilan
// belgilanadi — Flux'da `;` yo'q.

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    // Literallar
    Int(i64),
    Flt(f64),
    Str(Vec<StrPart>), // interpolatsiya bo'laklari bilan
    Sym(String),       // :ok  -> "ok"
    Ident(String),
    True,
    False,
    Nil,
    Inf, // cheksiz iterator (each i in inf)

    // Kalit so'zlar
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

    // Operatorlar va punktuatsiya
    Eq,        // =
    Assign,    // <-  (mutable bind / qayta tayinlash)
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
    Backslash, // \  (lambda boshi)
    Colon,     // :  (map kalit ajratuvchi)
    LParen,    // (
    RParen,    // )
    LBracket,  // [  (list ochuvchi)
    RBracket,  // ]
    LBrace,    // {  (map ochuvchi)
    RBrace,    // }
    Comma,     // , (Flux'da rasman yo'q, lekin xatoni aniqlash uchun ushlanadi)
    Spread,    // ... (map/list spread — round3 da qo'shilgan)

    // Tuzilma
    Newline,
    Indent,
    Dedent,
    Eof,
}

// String literal interpolatsiya bo'laklari: "salom ${name}!" ->
// [Lit("salom "), Expr("name"), Lit("!")]
#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    Lit(String),
    Expr(String), // ifoda manba matni; parser uni qayta parse qiladi
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    pub col: usize,
    // Bu token oldidan bo'shliq (yoki tab/newline) kelganmi. Grammatikada
    // `arr[i]` (tutash -> indeks) va `f "x" [a]` (bo'shliqli -> alohida argument)
    // ni ajratish uchun ishlatiladi.
    pub spaced: bool,
}
