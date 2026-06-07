// Flux frontend — ANALYZER (PR-4a): statik tahlil orqali view'ning qaysi qismi
// interaktiv (client island) yoki sof statik (SSR, 0 JS) ekanini aniqlaydi.
//
// Falsafa (docs/FRONTEND-PROD-ARCHITECTURE.md): dasturchi HECH NARSA belgilamaydi
// (Qwik `$`/Astro `client:load` emas) — analyzer "interaktivlik izi"ni AST'dan
// o'zi topadi. Flux ustunligi: reaktivlik belgisi (`<-`) tilning grammatikasida,
// shuning uchun "taxmin" qilish kerak emas (Marko'dan aniqroq).
//
// Bu modul SOF AST tahlil — interp ishtirok ETMAYDI (yon-effektsiz, har request
// emas, bir marta startup/build'da). Natija `ViewPlan`.
//
// PR-4a DOIRASI (birinchi qadam): view darajasida "interaktivmi yoki statikmi"
// aniqlash + bind klassifikatsiya (react/server/immutable) + server-only qiymat
// island'da ushlanishini tekshirish (qattiq xato). Element darajasidagi `island_id`
// va reaktivlik grafi keyingi PR (4b) — bu yerda asos quriladi.

#![allow(dead_code)] // ViewPlan maydonlari keyingi PR'larda (4b/5) ishlatiladi.

use std::collections::HashMap;

use crate::ast::{Expr, MapEntry, Program, Stmt, StrPiece};

// Bir bind (o'zgaruvchi/qiymat) qanday tabiatga ega.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindKind {
    // `x = expr` — o'zgarmas. Statik render uchun xavfsiz.
    Immut,
    // `x <- expr` — reaktiv state. Browserda o'zgaradi -> island izi.
    React,
    // `x = db.q ...` / `http.*` / `ai.*` — server-only. Client'ga ketmaydi.
    Server,
}

// Bitta view uchun tahlil natijasi.
#[derive(Debug, Clone)]
pub struct ViewPlan {
    pub name: String,
    // Bind nomi -> tabiati (Immut/React/Server).
    pub binds: HashMap<String, BindKind>,
    // View interaktivmi: `on:`/`bind:` props YOKI reaktiv (`<-`) state O'QILSA.
    // false -> sof statik (SSR, 0 JS, CDN-cacheable).
    pub interactive: bool,
}

impl ViewPlan {
    // React (`<-`) bilan e'lon qilingan bind nomlari.
    pub fn react_binds(&self) -> Vec<&str> {
        self.binds
            .iter()
            .filter(|(_, k)| **k == BindKind::React)
            .map(|(n, _)| n.as_str())
            .collect()
    }
    // Server-only bind nomlari.
    pub fn server_binds(&self) -> Vec<&str> {
        self.binds
            .iter()
            .filter(|(_, k)| **k == BindKind::Server)
            .map(|(n, _)| n.as_str())
            .collect()
    }
}

// Tahlil xatosi (qattiq: server-only qiymat island'da ushlansa). PR-4a'da xato
// turi tayyorlanadi; haqiqiy island-capture tekshiruvi 4b'da to'liq bo'ladi.
#[derive(Debug, Clone)]
pub struct AnalyzeError {
    pub view: String,
    pub message: String,
}

// Butun dasturdagi BARCHA view'larni tahlil qiladi -> nom bo'yicha ViewPlan.
pub fn analyze_program(prog: &Program) -> Result<HashMap<String, ViewPlan>, AnalyzeError> {
    let mut plans = HashMap::new();
    for stmt in prog {
        if let Stmt::ViewDecl { name, params, body } = stmt {
            let plan = analyze_view(name, params, body)?;
            plans.insert(name.clone(), plan);
        }
    }
    Ok(plans)
}

// Bitta view tanasini tahlil qiladi.
//
// Pass 1 — bindlarni klassifikatsiya qilish (= / <- / server-call).
// Pass 2 — interaktivlik izini topish (on:/bind: props, reaktiv state o'qilishi).
fn analyze_view(name: &str, params: &[String], body: &[Stmt]) -> Result<ViewPlan, AnalyzeError> {
    let mut binds: HashMap<String, BindKind> = HashMap::new();
    // Parametrlar (proplar) — tashqaridan keladi, statik deb hisoblanadi (Immut).
    for p in params {
        binds.insert(p.clone(), BindKind::Immut);
    }

    // Pass 1 — bindlar.
    collect_binds(body, &mut binds);

    // Pass 2 — interaktivlik izi.
    let interactive = body_has_interactivity(body, &binds);

    Ok(ViewPlan {
        name: name.to_string(),
        binds,
        interactive,
    })
}

// Pass 1: view tanasidagi bindlarni klassifikatsiya qiladi. Nested bloklar
// (each/if/match) ichidagi bindlar ham yig'iladi (transitiv).
fn collect_binds(stmts: &[Stmt], binds: &mut HashMap<String, BindKind>) {
    for s in stmts {
        match s {
            // `x = expr` — qiymat server-call bo'lsa Server, aks holda Immut.
            Stmt::Bind { name, value } => {
                let kind = if expr_is_server_call(value) {
                    BindKind::Server
                } else {
                    BindKind::Immut
                };
                binds.insert(name.clone(), kind);
            }
            // `x <- expr` — reaktiv state. (Birinchi e'lon REACT; keyingi
            // qayta-tayinlash ham REACT bo'lib qoladi.)
            Stmt::Assign { name, value: _ } => {
                binds.insert(name.clone(), BindKind::React);
            }
            // each/if/match — nested blok ichidagi bindlarni ham yig'amiz.
            Stmt::Each { body, .. } => collect_binds(body, binds),
            Stmt::Expr(e) => collect_binds_in_expr(e, binds),
            _ => {}
        }
    }
}

// Expr ichidagi blok-bindlarni yig'adi (if/match shoxlari Expr sifatida keladi).
fn collect_binds_in_expr(e: &Expr, binds: &mut HashMap<String, BindKind>) {
    match e {
        Expr::If(ifx) => {
            for (_, block) in &ifx.arms {
                collect_binds(block, binds);
            }
            if let Some(eb) = &ifx.else_block {
                collect_binds(eb, binds);
            }
        }
        Expr::Match(mx) => {
            for arm in &mx.arms {
                collect_binds(&arm.body, binds);
            }
        }
        _ => {}
    }
}

// Ifoda server-only chaqiruvmi (db.*/http.*/ai.*/reg.*)? Bu qiymatlar client'ga
// ketmaydi — natija (DTO) o'tishi mumkin, lekin ifodaning o'zi serverda qoladi.
fn expr_is_server_call(e: &Expr) -> bool {
    if let Expr::Call { callee, .. } = e
        && let Expr::Field { target, .. } = callee.as_ref()
        && let Expr::Ident(modname) = target.as_ref()
    {
        return is_server_module(modname);
    }
    false
}

// Server-only modul nomi (ma'lumot/secret manbai).
fn is_server_module(name: &str) -> bool {
    matches!(name, "db" | "http" | "ai" | "reg")
}

// Pass 2: view tanasida interaktivlik izi bormi.
//   - element propslarida `on:` yoki `bind:`  -> interaktiv
//   - reaktiv (`<-`) bind nomi O'QILSA (element ichida ishlatilsa) -> interaktiv
fn body_has_interactivity(stmts: &[Stmt], binds: &HashMap<String, BindKind>) -> bool {
    stmts.iter().any(|s| stmt_has_interactivity(s, binds))
}

fn stmt_has_interactivity(stmt: &Stmt, binds: &HashMap<String, BindKind>) -> bool {
    match stmt {
        Stmt::Expr(e) => expr_has_interactivity(e, binds),
        Stmt::Each { iter, body, .. } => {
            expr_has_interactivity(iter, binds) || body_has_interactivity(body, binds)
        }
        // bind/assign qiymatida reaktiv o'qish ham iz (masalan `total = qty * 2`
        // bu yerda qty reaktiv bo'lsa) — lekin asosiy iz element/props'da.
        Stmt::Bind { value, .. } | Stmt::Assign { value, .. } => {
            expr_has_interactivity(value, binds)
        }
        _ => false,
    }
}

fn expr_has_interactivity(e: &Expr, binds: &HashMap<String, BindKind>) -> bool {
    match e {
        // Reaktiv (`<-`) bind nomining O'QILISHI — eng muhim iz.
        Expr::Ident(name) => binds.get(name) == Some(&BindKind::React),
        // Element chaqiruvi: props map'ida `on:`/`bind:` bormi + argumentlar.
        Expr::Call { callee, args } => {
            expr_has_interactivity(callee, binds)
                || args
                    .iter()
                    .any(|a| map_has_event_or_bind(a) || expr_has_interactivity(a, binds))
        }
        // Map literal (props yoki oddiy): `on:`/`bind:` kalit -> interaktiv.
        Expr::Map(entries) => {
            map_entries_have_event_or_bind(entries)
                || entries.iter().any(|en| match en {
                    MapEntry::Pair { value, .. } => expr_has_interactivity(value, binds),
                    MapEntry::Dynamic { key, value } => {
                        expr_has_interactivity(key, binds) || expr_has_interactivity(value, binds)
                    }
                    MapEntry::Spread(ex) => expr_has_interactivity(ex, binds),
                })
        }
        Expr::List(items) => items.iter().any(|x| expr_has_interactivity(x, binds)),
        Expr::Field { target, .. } => expr_has_interactivity(target, binds),
        Expr::Index { target, key } => {
            expr_has_interactivity(target, binds) || expr_has_interactivity(key, binds)
        }
        Expr::Binary { lhs, rhs, .. } => {
            expr_has_interactivity(lhs, binds) || expr_has_interactivity(rhs, binds)
        }
        Expr::Unary { expr, .. } => expr_has_interactivity(expr, binds),
        Expr::Str(pieces) => pieces.iter().any(|p| match p {
            StrPiece::Expr(ex) => expr_has_interactivity(ex, binds),
            StrPiece::Lit(_) => false,
        }),
        Expr::If(ifx) => {
            ifx.arms.iter().any(|(cond, block)| {
                expr_has_interactivity(cond, binds) || body_has_interactivity(block, binds)
            }) || ifx
                .else_block
                .as_ref()
                .is_some_and(|eb| body_has_interactivity(eb, binds))
        }
        Expr::Match(mx) => {
            expr_has_interactivity(&mx.subject, binds)
                || mx
                    .arms
                    .iter()
                    .any(|arm| body_has_interactivity(&arm.body, binds))
        }
        _ => false,
    }
}

// Element props map'ida `on:` (event) yoki `bind:` (two-way) kaliti bormi.
// Parser props'ni `Expr::Map` sifatida beradi; kalitlar "on"/"bind".
fn map_has_event_or_bind(e: &Expr) -> bool {
    matches!(e, Expr::Map(entries) if map_entries_have_event_or_bind(entries))
}

fn map_entries_have_event_or_bind(entries: &[MapEntry]) -> bool {
    entries
        .iter()
        .any(|en| matches!(en, MapEntry::Pair { key, .. } if key == "on" || key == "bind"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    // Manbani parse qilib, birinchi view'ning planini qaytaradi.
    fn plan_of(src: &str, view: &str) -> ViewPlan {
        let toks = lex(src).expect("lex");
        let prog = parse(toks).expect("parse");
        let plans = analyze_program(&prog).expect("analyze");
        plans.get(view).expect("view topilmadi").clone()
    }

    #[test]
    fn sof_statik_view() {
        // Faqat literal/= -> interaktiv emas (sof SSR).
        let p = plan_of(
            r#"
view home
  h1 "Salom"
  p "xush kelibsiz"
"#,
            "home",
        );
        assert!(!p.interactive, "statik view interaktiv bo'lmasligi kerak");
    }

    #[test]
    fn on_event_interaktiv() {
        // on: event -> island.
        let p = plan_of(
            r#"
view counter
  n <- 0
  btn "+1" {on:save}
"#,
            "counter",
        );
        assert!(p.interactive, "on: bo'lgan view interaktiv");
        assert_eq!(p.binds.get("n"), Some(&BindKind::React));
    }

    #[test]
    fn reaktiv_oqish_interaktiv() {
        // <- state element ichida o'qiladi -> island.
        let p = plan_of(
            r#"
view c
  n <- 0
  p "Soni: $n"
"#,
            "c",
        );
        assert!(p.interactive, "reaktiv o'qish interaktiv qiladi");
    }

    #[test]
    fn bind_interaktiv() {
        // bind: two-way -> island.
        let p = plan_of(
            r#"
view form
  q <- ""
  input {bind:q}
"#,
            "form",
        );
        assert!(p.interactive, "bind: interaktiv qiladi");
    }

    #[test]
    fn server_call_bind() {
        // db.q natijasi -> Server bind; faqat statik o'qilsa view statik.
        let p = plan_of(
            r#"
view dash
  stats = db.q "select count(*) c from orders"
  p "Jami: ${stats.c}"
"#,
            "dash",
        );
        assert_eq!(p.binds.get("stats"), Some(&BindKind::Server));
        assert!(
            !p.interactive,
            "server data faqat o'qilsa view statik bo'ladi (0 JS)"
        );
    }

    #[test]
    fn each_ichida_reaktiv() {
        // each ichida reaktiv o'qish butun blokni interaktiv qiladi.
        let p = plan_of(
            r#"
view list items
  filter <- ""
  each it in items
    if it == filter
      p it
"#,
            "list",
        );
        assert!(
            p.interactive,
            "each ichida filter (react) o'qilsa interaktiv"
        );
    }

    #[test]
    fn parametr_immut() {
        // View parametri (prop) statik bo'lib o'qilsa interaktiv emas.
        let p = plan_of(
            r#"
view card title
  h2 title
"#,
            "card",
        );
        assert_eq!(p.binds.get("title"), Some(&BindKind::Immut));
        assert!(!p.interactive, "prop o'qish interaktiv qilmaydi");
    }
}
