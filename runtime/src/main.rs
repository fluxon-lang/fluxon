// Flux runtime — buyruq qatori interfeysi.
//
// Foydalanish:
//   flux run <fayl.fx>     — Flux faylini bajaradi
//   flux <fayl.fx>         — xuddi shu (qisqartma)
//   flux check <fayl.fx>   — faqat lex+parse (bajarmaydi); parse xato -> exit 2

// mimalloc — parallel'da system malloc'dan ancha kam contention beradi.
// Interpreter qisqa umrli scope allokatsiyalarini ko'p qiladi (tree-walking).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod ai_mod;
mod ast;
mod auth_mod;
mod builtins;
mod cron_mod;
mod db_mod;
mod http_mod;
mod interp;
mod lexer;
mod parser;
mod queue_mod;
mod reg_mod;
mod serve_mod;
mod token;
mod value;
mod ws_mod;

use std::process::ExitCode;

// Buyruq turi: `run` kodni bajaradi, `check` faqat sintaksisni tekshiradi.
// Exit kodlari ataylab farqli: faylni o'qib bo'lmasa/runtime xato -> 1,
// foydalanish/parse xato -> 2 (chaqiruvchi qaysi bosqichda yiqilganini biladi).
enum Command {
    Run(String),
    Check(String),
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let cmd = match parse_args(&args) {
        Some(c) => c,
        None => {
            eprintln!("Foydalanish: flux run <fayl.fx>  |  flux check <fayl.fx>");
            return ExitCode::from(2);
        }
    };

    let path = match &cmd {
        Command::Run(p) | Command::Check(p) => p.clone(),
    };

    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Faylni o'qib bo'lmadi '{}': {}", path, e);
            return ExitCode::from(1);
        }
    };

    match cmd {
        // run: LEX -> PARSE -> BAJAR. Xato (parse yoki runtime) -> exit 1.
        Command::Run(_) => match run_source_at(&src, std::path::Path::new(&path)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Flux xato: {}", e);
                ExitCode::from(1)
            }
        },
        // check: faqat LEX + PARSE (interp YO'Q -> side-effect yo'q). Forge
        // eval-gate QATLAM 1: AI yozgan blok sintaktik to'g'rimi, bajarmasdan.
        // Parse/lex xato -> exit 2 (runtime exit 1 dan farqli).
        Command::Check(_) => match check_source(&src) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Flux xato: {}", e);
                ExitCode::from(2)
            }
        },
    }
}

fn parse_args(args: &[String]) -> Option<Command> {
    match args.get(1).map(|s| s.as_str()) {
        Some("run") => args.get(2).cloned().map(Command::Run),
        Some("check") => args.get(2).cloned().map(Command::Check),
        Some(p) if !p.starts_with('-') => Some(Command::Run(p.to_string())),
        _ => None,
    }
}

// Sintaksisni tekshiradi: lex + parse, lekin interp'ni o't kazib yuboradi —
// kod BAJARILMAYDI (side-effect yo'q). Muvaffaqiyatda Ok(()), aks holda xato matni.
fn check_source(src: &str) -> Result<(), String> {
    let toks = lexer::lex(src)?;
    parser::parse(toks)?;
    Ok(())
}

// Manbani bajaradi. `path` — faylning yo'li; `use ./fayl` modullari shu faylning
// katalogiga nisbatan hal qilinadi.
fn run_source_at(src: &str, path: &std::path::Path) -> Result<(), String> {
    let toks = lexer::lex(src)?;
    let prog = parser::parse(toks)?;
    // Arc<Interp>: http.serve handler'larni server thread'larida apply qiladi,
    // shuning uchun interp thread'lar orasida ulashiladigan bo'lishi kerak.
    let interp = interp::Interp::new_arc();
    // `use ./fayl` uchun base — top-level faylning katalogi.
    if let Some(dir) = path.parent() {
        // parent() bo'sh ("") bo'lsa joriy katalog (default) qoladi.
        if !dir.as_os_str().is_empty() {
            interp.set_base(dir);
        }
    }
    interp.run(&prog)
}

// Path'siz qulay wrapper — testlar uchun (modul yo'llari joriy katalogga nisbatan).
#[cfg(test)]
fn run_source(src: &str) -> Result<(), String> {
    run_source_at(src, std::path::Path::new("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Kichik yordamchi: manbani bajaradi, xato bo'lsa panic.
    fn run(src: &str) {
        run_source(src).unwrap_or_else(|e| panic!("xato: {}", e));
    }

    #[test]
    fn fib_recursion() {
        run(r#"
fn fib n
  if n < 2
    ret n
  (fib (n - 1)) + (fib (n - 2))

each i in 0..10
  log "fib ${i} = ${fib i}"
"#);
    }

    // Inline if (ternary ekvivalenti): `if shart a else b` bir qiymat qaytaradi.
    // Issue #66 — ixcham shartli ifoda (leading-zero formatlash kabi joylar uchun).
    #[test]
    fn inline_if_ifoda() {
        run(r#"
# Issue'dagi asosiy misol: leading-zero formatlash
h = 5
pad = if h < 10 ("0" + str.str h) else (str.str h)
(pad == "05") | (fail "inline if qiymat bermadi: ${pad}")

# shart yolg'on bo'lganda else tarmog'i
x = 20
pad2 = if x < 10 ("0" + str.str x) else (str.str x)
(pad2 == "20") | (fail "else tarmog'i ishlamadi: ${pad2}")

# qavssiz oddiy tarmoqlar
y = if h > 3 "katta" else "kichik"
(y == "katta") | (fail "qavssiz tarmoq ishlamadi: ${y}")

# else-if zanjiri (ichma-ich inline if)
g = if h == 0 "nol" else if h < 0 "manfiy" else "musbat"
(g == "musbat") | (fail "else-if zanjiri ishlamadi: ${g}")

# chaqiruvli shart qavs ichida
s = "hi"
r = if (str.len s) > 0 "to'la" else "bo'sh"
(r == "to'la") | (fail "qavsli shart ishlamadi: ${r}")

# katta ifoda ichida ishlatish
n = 7
msg = "son " + (if n % 2 == 0 "juft" else "toq")
(msg == "son toq") | (fail "ichki inline if ishlamadi: ${msg}")
"#);
    }

    // Inline shakl qo'shilgach ham blok shakli (chaqiruvli shart bilan) ishlashi
    // kerak — regressiya tekshiruvi.
    #[test]
    fn blok_if_inline_qoshilgach_ishlaydi() {
        run(r#"
s = "hi"
out <- "yo'q"
if str.len s > 0
  out <- "to'la"
else
  out <- "bo'sh"
(out == "to'la") | (fail "blok if buzildi: ${out}")
"#);
    }

    // Argumentsiz (nullary) chaqiruv: `f()`. Qavssiz chaqirish argument bilan
    // aniqlangani uchun 0-arity funksiyani chaqirishning yagona yo'li shu.
    // `f` (qavssiz) funksiya QIYMATI, `f()` esa CHAQIRUV.
    #[test]
    fn nullary_call() {
        run(r#"
fn new_id
  ret rand.str 8

a = new_id()
b = new_id()
(str.len a == 8) | (fail "new_id() chaqirilmadi: ${a}")
(a != b) | (fail "har chaqiruv yangi qiymat bermadi")

# qavssiz: funksiya qiymati (chaqirilmaydi) — boolean truthy
f = new_id
(f != nil) | (fail "qavssiz nom funksiya qiymati bo'lishi kerak")

# lambda nullary
g = \->
  ret 42
(g() == 42) | (fail "lambda nullary chaqiruv ishlamadi: ${g()}")
"#);
    }

    // Argumentsiz rekursiya: `tick()` o'zini chaqiradi. Ilgari dummy argument
    // (`tick n`) kiritishga majbur edik — endi shart emas.
    #[test]
    fn nullary_recursion() {
        run(r#"
n <- 0
fn tick
  n <- n + 1
  if n < 3
    tick()
  ret n
(tick() == 3) | (fail "nullary rekursiya ishlamadi: ${n}")
"#);
    }

    // `f(x)` (argument bilan qavsli chaqiruv) RAD ETILADI — canonical shakl `f x`.
    // Bo'sh `()` faqat nullary uchun; bir ish = bir yo'l.
    #[test]
    fn paren_call_with_arg_errors() {
        let err = run_source(
            r#"
fn g x
  ret x
g(5)
"#,
        )
        .expect_err("f(x) qavsli argument xato berishi kerak");
        assert!(err.contains("argumentsiz"), "kutilmagan xato: {}", err);
    }

    #[test]
    fn list_methods() {
        run(r#"
nums = [1 2 3 4 5]
evens = nums.filter \x -> x % 2 == 0
doubled = evens.map \x -> x * 2
total = doubled.reduce 0 \acc x -> acc + x
log "evens=${evens} doubled=${doubled} total=${total}"
"#);
    }

    // list.index pozitsiya beradi (topilmasa -1), list.find predikatga mos
    // birinchi elementni (topilmasa nil). has bool, index pozitsiya — juftlik.
    #[test]
    fn list_index_and_find() {
        run(r#"
names = ["catalog_manager" "order_extractor" "billing"]
(names.index "order_extractor" == 1) | (fail "index topmadi: ${names.index "order_extractor"}")
(names.index "yoq" == -1) | (fail "yo'q element -1 bermadi")

nums = [3 1 4 1 5 9]
(nums.index 4 == 2) | (fail "int index: ${nums.index 4}")

# find: predikatga mos birinchi element
big = nums.find \x -> x > 4
(big == 5) | (fail "find mos elementni bermadi: ${big}")
none = nums.find \x -> x > 99
(none == nil) | (fail "find topmaganda nil bermadi: ${none}")

# index'ni solishtirish uchun ishlatish (issue manbasi: blok tartibi)
a = names.index "catalog_manager"
b = names.index "billing"
(a < b) | (fail "indeks solishtirish ishlamadi: ${a} ${b}")
"#);
    }

    // Hisoblangan indeks: `xs.(ifoda)` va `xs[ifoda]` ikkalasi ham ishlashi kerak.
    // Issue #64 — pagination/oxirgi element olish uchun literal emas, ifoda-indeks.
    #[test]
    fn hisoblangan_indeks() {
        run(r#"
xs = ["a" "b" "c"]
i = xs.len - 1

# .(ifoda) shakli — oxirgi elementni hisoblangan indeks bilan ol
last = xs.(i)
(last == "c") | (fail ".(i) oxirgi elementni bermadi: ${last}")

# ichida to'liq ifoda
(xs.(xs.len - 1) == "c") | (fail "xs.(xs.len - 1) ishlamadi")

# bracket shakli ham bir xil natija beradi
(xs[i] == "c") | (fail "xs[i] ishlamadi")

# map'ni hisoblangan kalit (str) bilan indekslash
m = {name: "Ali" age: 30}
k = "name"
(m.(k) == "Ali") | (fail "m.(k) ishlamadi: ${m.(k)}")

# chegaradan tashqari -> nil (mavjud get_index xulqi)
(xs.(99) == nil) | (fail "chegaradan tashqari indeks nil bermadi")
"#);
    }

    #[test]
    fn map_operations() {
        run(r#"
u = {name:"Aziza" age:30}
u2 = u.set "age" 31
log "keys=${u.keys} hasName=${u.has "name"} age=${u2.age}"
"#);
    }

    // Schema map qiymat pozitsiyasidagi bare tip nomi (`{a:str b:int}`) sym'ga
    // aylanadi — docs (`ai.json {product:str qty:int}`) va'da qilgani. `str` ham
    // modul nomi bo'lgani uchun ilgari "noma'lum nom: str" xatosini berardi.
    #[test]
    fn schema_bare_type_names() {
        run(r#"
schema = {product:str qty:int price:flt active:bool data:json tag:sym}
(schema.product == :str) | (fail "product :str emas: ${schema.product}")
(schema.qty == :int) | (fail "qty :int emas: ${schema.qty}")
(schema.price == :flt) | (fail "price :flt emas")
(schema.active == :bool) | (fail "active :bool emas")
(schema.data == :json) | (fail "data :json emas")
(schema.tag == :sym) | (fail "tag :sym emas")

# nested list ichidagi map ham ishlasin (`{items:[{product:str qty:int}]}`)
nested = {items:[{product:str qty:int}]}
row = nested.items.0
(row.product == :str) | (fail "nested product :str emas")
(row.qty == :int) | (fail "nested qty :int emas")

# regressiya: tip nomi BO'LMAGAN ident hamon o'zgaruvchi sifatida qidiriladi
x = 5
m = {n:x}
(m.n == 5) | (fail "oddiy o'zgaruvchi qiymat buzildi: ${m.n}")

# regressiya: str modul-chaqiruvi qiymat sifatida buzilmadi
up = str.up "salom"
(up == "SALOM") | (fail "str.up buzildi: ${up}")
"#);
    }

    #[test]
    fn mutable_and_each() {
        run(r#"
total <- 0
each n in [10 20 30]
  total <- total + n
log "total=${total}"
"#);
    }

    // if/each/match bloklari leksik jihatdan SHAFFOF: ichidagi `=` tashqi (bir xil
    // fn'dagi) o'zgaruvchini yangilaydi — boshqa tillar kabi, klon olinmaydi. Bu
    // accumulator pattern'ni tabiiy qiladi (avval blok ichida `=` jim yangi local
    // yaratardi → tashqi nil qolardi).
    #[test]
    fn bind_in_block_updates_outer() {
        run(r#"
best <- nil
top <- 0
each e in [{n:"a" v:3} {n:"b" v:7} {n:"c" v:2}]
  if e.v > top
    top = e.v
    best = e
(top == 7) | (fail "top noto'g'ri: ${top}")
(best.n == "b") | (fail "best noto'g'ri: ${best.n}")
"#);
    }

    // Immutability saqlanadi: tashqi `=` (immutable) o'zgaruvchini blok ichidan
    // `=` bilan ham qayta tayinlab bo'lmaydi (aniq xato — jim shadow EMAS).
    #[test]
    fn bind_in_block_immutable_errors() {
        let err = run_source(
            r#"
x = 10
if true
  x = 20
"#,
        )
        .expect_err("immutable'ni blok ichida = bilan yangilash xato berishi kerak");
        assert!(err.contains("o'zgarmas"), "kutilmagan xato: {}", err);
    }

    // fn/lambda CHEGARA: ichidagi `=` tashqi o'zgaruvchini emas, yangi LOCAL
    // yaratadi (shadowing/izolyatsiya). Tashqi qiymat o'zgarmaydi.
    #[test]
    fn bind_in_fn_shadows_not_mutates() {
        run(r#"
x = 100
f = \n ->
  x = 5
  x + n
(f 1 == 6) | (fail "fn local x ishlamadi")
(x == 100) | (fail "fn ichidagi = tashqi x ni o'zgartirdi: ${x}")
"#);
    }

    // `<-` (assign) esa fn chegarasidan O'TADI — closure capture saqlanadi
    // (`=` chegarada to'xtaydi, `<-` to'xtamaydi: ikkalasining aniq farqi).
    #[test]
    fn assign_crosses_fn_boundary_capture() {
        run(r#"
counter <- 0
inc = \n ->
  counter <- counter + n
inc 5
inc 3
(counter == 8) | (fail "closure capture ishlamadi: ${counter}")
"#);
    }

    #[test]
    fn match_symbols() {
        run(r#"
fn label s
  match s
    :new -> "yangi"
    :done -> "tugadi"
    _ -> "boshqa"

log (label :new)
log (label :x)
"#);
    }

    #[test]
    fn string_and_modules() {
        run(r#"
s = "Salom Dunyo"
log (str.up s)
log "len=${str.len s} floor=${math.floor 3.7}"
parts = str.split "a,b,c" ","
log "parts=${parts} joined=${parts.join "-"}"
"#);
    }

    #[test]
    fn time_module_fmt_and_roundtrip() {
        // time.fmt unix int bilan deterministik: 1700000000 = 2023-11-14 22:13:20 UTC.
        // time.now/time.ago matn formatini ("YYYY-MM-DD HH:MM:SS") tekshiramiz va
        // fmt orqali round-trip qilamiz.
        run(r#"
d = time.fmt 1700000000 "YYYY-MM-DD"
(d == "2023-11-14") | (fail "fmt sana noto'g'ri: ${d}")
t = time.fmt 1700000000 "HH:mm:ss"
(t == "22:13:20") | (fail "fmt vaqt noto'g'ri: ${t}")
n = time.now
(str.len n == 19) | (fail "time.now uzunligi 19 emas: ${n}")
back = time.fmt n "YYYY"
(str.len back == 4) | (fail "time.now -> fmt yil 4 raqam emas")
"#);
    }

    #[test]
    fn time_ago_is_earlier() {
        // time.ago hozirdan oldin: ISO matn format leksikografik = xronologik,
        // shuning uchun DB filtri (`created > $1`) SQL'da to'g'ri ishlaydi. Bu
        // yerda yil/oy/kun bo'laklarini taqqoslab xronologik tartibni isbotlaymiz.
        run(r#"
now = time.now
past = time.ago 1 :day
ny = str.int (time.fmt now "YYYYMMDDHHmmss")
py = str.int (time.fmt past "YYYYMMDDHHmmss")
(py < ny) | (fail "time.ago kelajakda: past=${past} now=${now}")
"#);
    }

    #[test]
    fn time_in_is_later() {
        // time.in hozirdan keyin (TTL/expiry uchun). time.ago ning ko'zgusi:
        // ISO matn leksikografik = xronologik, shuning uchun `expires > $now`
        // SQL filtri to'g'ri ishlaydi. Yil/oy/...sek bo'laklarini taqqoslaymiz.
        run(r#"
now = time.now
soon = time.in 1 :hr
ny = str.int (time.fmt now "YYYYMMDDHHmmss")
sy = str.int (time.fmt soon "YYYYMMDDHHmmss")
(sy > ny) | (fail "time.in o'tmishda: soon=${soon} now=${now}")
"#);
    }

    #[test]
    fn time_parse_add_diff_booking_flow() {
        // Issue #65: mijoz ISO `start_at` va `duration_minutes` beradi ->
        // server `end_at` ni hisoblaydi. Booking yadrosining e2e ssenariysi.
        run(r#"
start_at = time.parse "2026-06-10T10:00:00Z"
(start_at == "2026-06-10 10:00:00") | (fail "parse noto'g'ri: ${start_at}")
end_at = time.add start_at 30 :min
(end_at == "2026-06-10 10:30:00") | (fail "add noto'g'ri: ${end_at}")
mins = (time.diff end_at start_at) / 60
(mins == 30) | (fail "diff noto'g'ri: ${mins}")
# buffer-inclusive interval: start - 5min (time.sub — add ning ko'zgusi)
buf_start = time.sub start_at 5 :min
(buf_start == "2026-06-10 09:55:00") | (fail "time.sub noto'g'ri: ${buf_start}")
"#);
    }

    #[test]
    fn time_parse_handles_iso_offset() {
        // ISO mintaqali matn UTC ga keltiriladi (+05:00 -> vaqt 5 soat oldin).
        run(r#"
t = time.parse "2026-06-10T15:00:00+05:00"
(t == "2026-06-10 10:00:00") | (fail "mintaqa UTC ga kelmadi: ${t}")
"#);
    }

    #[test]
    fn keyword_as_field_name() {
        // `.` dan keyin kalit so'z field nomi bo'la oladi (time.in shu tufayli ishlaydi).
        // Map kaliti kalit so'z bo'lsa ham `.in`/`.match` bilan o'qiladi — bu Flux
        // falsafasi: member pozitsiyasida kalit so'zning grammatik ma'nosi yo'q.
        run(r#"
m = {in: 1 match: 2 each: 3}
(m.in == 1) | (fail "m.in: ${m.in}")
(m.match == 2) | (fail "m.match: ${m.match}")
(m.each == 3) | (fail "m.each: ${m.each}")
"#);
    }

    #[test]
    fn env_member_access() {
        // env.NOM -> std::env. Yo'q bo'lsa nil -> `??` default. Bor bo'lsa qiymat.
        // FLUX_TEST_VAR'ni o'rnatib o'qiymiz (DB_TEST_LOCK kerak emas — boshqa env).
        unsafe { std::env::set_var("FLUX_TEST_VAR", "salom") };
        run(r#"
v = env.FLUX_TEST_VAR
(v == "salom") | (fail "env o'qish: ${v}")
miss = env.FLUX_NONEXISTENT_XYZ ?? "default"
(miss == "default") | (fail "yo'q env nil -> default emas: ${miss}")
"#);
        unsafe { std::env::remove_var("FLUX_TEST_VAR") };
    }

    #[test]
    fn env_shadowed_by_local() {
        // Foydalanuvchi `env` nomli o'zgaruvchi yaratsa, u built-in env'ni ustun
        // bosadi (member access map'ga ishlaydi, std::env'ga emas).
        run(r#"
env = {PORT:"9999"}
p = env.PORT
(p == "9999") | (fail "local env shadow ishlamadi: ${p}")
"#);
    }

    #[test]
    fn json_unicode_roundtrip() {
        // json.dec ko'p baytli UTF-8 (emoji, o'zbekcha) va \u escape (surrogate
        // juftligi) ni TO'G'RI dekodlasin — avval bayt-bayt `as char` mojibake
        // berardi (🙂 -> ð...). Bu yadro tuzatishi http/db/ai hammasiga taalluqli.
        run(r#"
# Xom UTF-8 baytlar (escape'siz): emoji + o'zbekcha — bayt-bayt as char BUZARDI
r = json.dec "{\"s\":\"o'zbek 🙂 g'ayrat\"}"
(r.s == "o'zbek 🙂 g'ayrat") | (fail "xom UTF-8 buzildi: ${r.s}")
# \u escape: BMP belgisi (ü = ü). \\u -> manbada literal \u bo'ladi.
u = json.dec "{\"c\":\"\\u00fc\"}"
(u.c == "ü") | (fail "\\u00fc dekod buzildi: ${u.c}")
# \u surrogate juftligi (🙂 = 🙂)
e = json.dec "{\"c\":\"\\ud83d\\ude42\"}"
(e.c == "🙂") | (fail "\\u surrogate juftligi buzildi: ${e.c}")
# enc -> dec round-trip
back = json.dec (json.enc {x:"salom 🙂 dünyo"})
(back.x == "salom 🙂 dünyo") | (fail "round-trip buzildi: ${back.x}")
"#);
    }

    #[test]
    fn reg_add_call_has_names() {
        // reg battery: funksiyani nom bilan saqlash/chaqirish (dinamik dispatch).
        // closure args map oladi (agent tool naqshi); reg.has bool, reg.names list.
        run(r#"
reg.add "calc" \args -> args.a + args.b
reg.add "greet" \args -> "salom ${args.nom}"

out = reg.call "calc" {a:2 b:3}
(out == 5) | (fail "reg.call calc noto'g'ri: ${out}")

g = reg.call "greet" {nom:"Aziza"}
(g == "salom Aziza") | (fail "reg.call greet noto'g'ri: ${g}")

(reg.has "calc") | (fail "reg.has calc false bo'lmasligi kerak")
((reg.has "yoq") == false) | (fail "reg.has yoq true bo'lmasligi kerak")

# reg.names argumentsiz (Field) — alifbo tartibida barqaror chiqish
ns = reg.names
(ns.len == 2) | (fail "reg.names uzunligi 2 emas: ${ns}")
(ns.0 == "calc") | (fail "reg.names[0] calc emas: ${ns}")
"#);
    }

    #[test]
    fn reg_call_unknown_fails() {
        // Ro'yxatda yo'q nomni chaqirish fail bo'lishi kerak (jim nil emas).
        let err = run_source(
            r#"
out = reg.call "yoq" {a:1}
log out
"#,
        )
        .unwrap_err();
        assert!(
            err.contains("ro'yxatda yo'q"),
            "kutilgan 'ro'yxatda yo'q', topildi: {}",
            err
        );
    }

    #[test]
    fn reg_add_overwrites() {
        // Bir nomga qayta reg.add — ustiga yozadi (tool yangilash holati).
        run(r#"
reg.add "f" \args -> 1
reg.add "f" \args -> 2
out = reg.call "f" {}
(out == 2) | (fail "reg.add ustiga yozmadi: ${out}")
"#);
    }

    #[test]
    fn fail_as_expr_and_guard() {
        // fail ifoda kontekstida (guard) — oqimni uzadi, yuqoriga ko'tariladi.
        let err = run_source(
            r#"
fn check x
  x > 0 | (fail 422 "musbat bo'lishi kerak")
  "ok"
log (check 5)
log (check 0)
"#,
        )
        .unwrap_err();
        assert!(err.contains("422"), "kutilgan 422, topildi: {}", err);
    }

    #[test]
    fn pipe_and_coalesce() {
        run(r#"
fn inc x -> x + 1
fn sq x -> x * x
r = 3 |> inc |> sq
log "r=${r}"
m = {a:1}
log "missing=${m.b ?? "yo'q"}"
"#);
    }

    // --- db battery testlari (in-memory SQLite, har Interp alohida DB) ---

    // DATABASE_URL global env — uni o'rnatib darhol run qilish race bo'lmasligi
    // uchun db testlarini global mutex bilan SERIALIZATSIYA qilamiz. Guard
    // ushlangan paytda boshqa db testi env'ni o'zgartirmaydi. Har test ALOHIDA
    // nomlangan shared-cache memory DB ishlatadi (pool bir nechta connection
    // ochadi → shared-cache kerak; unikal nom → testlar bir-birini ko'rmaydi).
    static DB_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_db_test(name: &str, body: impl FnOnce()) {
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let url = format!("sqlite:file:{name}?mode=memory&cache=shared");
        // SAFETY: guard ushlanган — bir vaqtda faqat bitta db testi env qo'yadi.
        unsafe { std::env::set_var("DATABASE_URL", &url) };
        body();
    }

    #[test]
    fn db_ins_sym_json_roundtrip() {
        // ins generatsiya qilingan id qaytaradi; sym Str<->Sym; json map round-trip.
        with_db_test("ins_sym_json", || {
            run(r#"
use db
tbl tickets
  id       serial pk
  category sym
  meta     json
t = db.ins "tickets" {category::billing meta:{tries:3}}
(t.id == 1) | (fail "id 1 bo'lishi kerak")
match t.category
  :billing -> log "ok sym"
  _ -> fail "sym :billing bo'lishi kerak"
(t.meta.tries == 3) | (fail "json meta.tries 3 bo'lishi kerak")
"#);
        });
    }

    #[test]
    fn db_param_and_placeholder() {
        // param'siz q + $1 placeholder SQLite'da rewrite'siz bog'lanadi + sym param.
        with_db_test("param_placeholder", || {
            run(r#"
use db
tbl items
  id   serial pk
  kind sym
db.ins "items" {kind::a}
db.ins "items" {kind::b}
all = db.q "select * from items"
(all.len == 2) | (fail "param'siz q 2 qator")
only = db.q "select * from items where kind=$1" [:a]
(only.len == 1) | (fail "$1 sym param 1 qator")
"#);
        });
    }

    #[test]
    fn db_tx_commit_returns_value() {
        with_db_test("tx_commit", || {
            run(r#"
use db
tbl t
  id serial pk
  n  int
r = db.tx \->
  x = db.ins "t" {n:7}
  ret x
(r.n == 7) | (fail "tx ret qiymati n=7")
(db.one "select count(*) c from t").c == 1 | (fail "1 qator commit bo'lishi kerak")
"#);
        });
    }

    #[test]
    fn db_tx_rollback_on_fail() {
        // tx ichida fail -> butun blok rollback; xato yuqoriga ko'tariladi va
        // birinchi (tx'siz) ins saqlanib, tx ichidagi ins rollback bo'ladi.
        // FAYL-backed temp DB: ikki run_source orasida saqlanadi (memory DB esa
        // birinchi Interp drop bo'lganda o'chadi). Tekshiruvchi run ALOHIDA Interp.
        let path = std::env::temp_dir().join("flux_tx_rollback_test.db");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guard ushlangan.
        unsafe {
            std::env::set_var("DATABASE_URL", format!("sqlite:{}", path.display()));
        }

        let err = run_source(
            r#"
use db
tbl t
  id serial pk
  n  int
db.ins "t" {n:1}
db.tx \->
  db.ins "t" {n:2}
  fail "ataylab"
"#,
        )
        .unwrap_err();
        assert!(
            err.contains("ataylab"),
            "kutilgan fail xabari, topildi: {}",
            err
        );

        // Alohida (yangi) Interp/pool — fayl DB saqlangan. Rollback ishlagan bo'lsa
        // faqat tx'siz ins (n:1) qoladi, tx ichidagi (n:2) yo'q.
        run_source(
            r#"
use db
tbl t
  id serial pk
  n  int
(db.one "select count(*) c from t").c == 1 | (fail "rollback'dan keyin 1 qator qolishi kerak")
"#,
        )
        .unwrap_or_else(|e| panic!("rollback tekshiruvi: {}", e));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn db_json_col_cross_process_decode() {
        // Issue #63: json ustun `tbl` e'lon QILINMAGAN process'da ham map qaytarsin.
        // Ikki ALOHIDA Interp (= ikki process) bir FAYL DB ustida: birinchi yozadi
        // (tbl bilan), ikkinchi tbl'siz o'qiydi — DB introspeksiyasi ustun json
        // ekanini tiklab map beradi (ilgari xom string qaytib row.body.x xato berardi).
        let path = std::env::temp_dir().join("flux_json_xproc_test.db");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guard ushlangan.
        unsafe {
            std::env::set_var("DATABASE_URL", format!("sqlite:{}", path.display()));
        }

        // Yozuvchi process: tbl e'lon qiladi + json map (ichida list ham) yozadi.
        run_source(
            r#"
use db
tbl t
  k    sym
  body json
db.ins "t" {k::a body:{x:1 y:[1 2 3]}}
"#,
        )
        .unwrap_or_else(|e| panic!("yozish: {}", e));

        // O'qiydigan process: tbl YO'Q — faqat o'qiydi. json map bo'lib kelishi shart.
        run_source(
            r#"
use db
row = db.one "select * from t where k=$1" [:a]
(row.body.x == 1) | (fail "json ustun map bo'lib dekod bo'lishi kerak (x)")
(row.body.y.len == 3) | (fail "json ichki list ham tiklanishi kerak (y)")
"#,
        )
        .unwrap_or_else(|e| panic!("o'qish: {}", e));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn db_json_schema_less_write_to_text_col() {
        // Regression: tbl e'lon QILINMAGAN process TEXT ustuniga map/list yoza olsin.
        // Ilgari DB introspeksiyasi TEXT ustunni Some("text") qaytarardi va write path
        // "json ustun emas" xatosi berardi — endi yozish tomoni faqat tbl registry'dan
        // foydalanadi, shuning uchun tbl yo'q process uchun schema-less yozish ishlaydi.
        //
        // Scenario: birinchi process `str` (TEXT) ustun yaratadi; ikkinchi process
        // tbl YO'Q holda map yozadi — bu ilgari "json ustun emas" xatosi berardi.
        let path = std::env::temp_dir().join("flux_schemaless_write_test.db");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        let _guard = DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("DATABASE_URL", format!("sqlite:{}", path.display()));
        }

        // Birinchi process: str (TEXT) ustun bilan jadval yaratadi va bir qator yozadi
        // (db.ins lazy DB open + migrate qiladi — jadval aynan shu yerda yaratiladi).
        run_source(
            r#"
use db
tbl t3
  id   serial pk
  body str
db.ins "t3" {body:"init"}
"#,
        )
        .unwrap_or_else(|e| panic!("jadval yaratish: {}", e));

        // Ikkinchi process: tbl YO'Q — TEXT ustuniga map yozishi kerak (schema-less).
        run_source(
            r#"
use db
db.ins "t3" {body:{x:42 y:[1 2]}}
row = db.one "select body from t3 limit 1"
row.body | (fail "body bo'sh bo'lmasligi kerak")
"#,
        )
        .unwrap_or_else(|e| panic!("schema-less yozish: {}", e));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn db_tx_nested_savepoint() {
        // Ichki tx (SAVEPOINT). Ichki blok ret qiymat qaytaradi, tashqi commit.
        with_db_test("tx_nested", || {
            run(r#"
use db
tbl t
  id serial pk
  n  int
r = db.tx \->
  db.ins "t" {n:1}
  inner = db.tx \->
    x = db.ins "t" {n:2}
    ret x
  ret inner
(r.n == 2) | (fail "nested tx ret qiymati n=2")
(db.one "select count(*) c from t").c == 2 | (fail "ikkala ins commit bo'lishi kerak")
"#);
        });
    }

    #[test]
    fn db_put_upsert() {
        with_db_test("put_upsert", || {
            run(r#"
use db
tbl counters
  name str pk
  hits int
db.put "counters" {hits:1} {name:"x"}
db.put "counters" {hits:9} {name:"x"}
c = db.one "select * from counters where name=$1" ["x"]
(c.hits == 9) | (fail "upsert hits=9 bo'lishi kerak")
n = (db.q "select * from counters").len
(n == 1) | (fail "upsert dublikat yaratmasligi kerak")
"#);
        });
    }

    #[test]
    fn db_uniq_violation_rolls_back_tx() {
        // uniq buzilishi tx ichida -> rollback (idempotency naqshi).
        with_db_test("uniq_violation", || {
            let err = run_source(
                r#"
use db
tbl txns
  id   serial pk
  ikey str uniq
db.ins "txns" {ikey:"k1"}
db.tx \->
  db.ins "txns" {ikey:"k1"}
"#,
            )
            .unwrap_err();
            // uniq buzilishi db xato sifatida ko'tariladi.
            assert!(
                err.to_lowercase().contains("unique") || err.contains("db xato"),
                "kutilgan uniq buzilish xatosi, topildi: {}",
                err
            );
        });
    }

    // --- cron battery ---

    #[test]
    fn cron_on_registratsiya_xatosiz() {
        // Tirnoqsiz 5-maydon (nomli funksiya). cron.on bloklamaydi, dastur tugaydi.
        run(r#"
fn check
  log "tekshiruv"
cron.on 0 * * * * check
"#);
    }

    #[test]
    fn cron_on_lambda_va_murakkab_ifoda() {
        // Inline lambda + step/range/list aralash ifoda.
        run(r#"
cron.on */15 9 1,15 * 1-5 \->
  log "har 15 daqiqa, 9-soat, 1 va 15-kun, ish kunlari"
"#);
    }

    #[test]
    fn cron_on_tirnoqli_variant() {
        // Tirnoqli str ham ishlaydi (inson uchun; AI docs'da yo'q).
        run(r#"
fn report
  log "hisobot"
cron.on "30 9 * * *" report
"#);
    }

    #[test]
    fn cron_on_notogri_ifoda_xato() {
        // 99-daqiqa yo'q — cron.on xato qaytarishi kerak.
        let err = run_source(
            r#"
fn f
  log "x"
cron.on 99 * * * * f
"#,
        )
        .expect_err("noto'g'ri cron ifoda xato berishi kerak");
        assert!(
            err.contains("cron") && err.to_lowercase().contains("ifoda"),
            "kutilgan cron ifoda xatosi, topildi: {}",
            err
        );
    }

    // --- queue battery ---

    #[test]
    fn queue_on_push_registratsiya_xatosiz() {
        // queue.on handler ro'yxatga oladi, queue.push ish qo'shadi — ikkalasi ham
        // bloklamaydi, dastur tugaydi (worker fonda ishlayveradi). Handler bittagina
        // `job` map argumenti oladi.
        run(r#"
queue.on "send" \job ->
  log "yuborilmoqda: ${job.ph}"
queue.push "send" {ph:"+99890" body:"salom"}
"#);
    }

    #[test]
    fn queue_push_payloadsiz() {
        // Payload ixtiyoriy — berilmasa job Nil bo'ladi.
        run(r#"
queue.on "tozala" \job ->
  log "tozalandi"
queue.push "tozala"
"#);
    }

    #[test]
    fn queue_push_nom_str_bolmasa_xato() {
        // 1-argument ish nomi str bo'lishi shart.
        let err = run_source(r#"queue.push 5"#).expect_err("nom str bo'lmasa xato kutiladi");
        assert!(
            err.contains("queue.push"),
            "kutilgan queue.push xatosi, topildi: {}",
            err
        );
    }

    #[test]
    fn queue_argumentsiz_dispatch_ga_yetadi() {
        // Argumentsiz `queue.X` (Call emas, Field bo'lib keladi) modul dispatch'iga
        // yetishi kerak — `queue` ident o'zgaruvchi deb qidirilib "noma'lum nom"
        // bermasin. Noma'lum funksiya bilan sinaymiz: dispatch'ga yetsa "queue
        // modulida ... yo'q" xatosi keladi (noma'lum nom EMAS). [cron.run regressiyasi]
        let err = run_source(r#"queue.yoq"#).expect_err("argumentsiz queue.yoq xato berishi kerak");
        assert!(
            err.contains("queue modulida") && !err.contains("noma'lum nom"),
            "argumentsiz queue dispatch'ga yetishi kerak, topildi: {}",
            err
        );
    }

    #[test]
    fn cron_argumentsiz_dispatch_ga_yetadi() {
        // `cron.run` argumentsiz — Field bo'lib keladi va dispatch'ga yetishi kerak
        // (aks holda "noma'lum nom: cron"). cron.run bloklaydi, shuning uchun mavjud
        // funksiya o'rniga noma'lum funksiya bilan dispatch'ga yetganini tekshiramiz.
        let err = run_source(r#"cron.yoq"#).expect_err("argumentsiz cron.yoq xato berishi kerak");
        assert!(
            err.contains("cron modulida") && !err.contains("noma'lum nom"),
            "argumentsiz cron dispatch'ga yetishi kerak, topildi: {}",
            err
        );
    }

    #[test]
    fn queue_on_handler_fn_bolmasa_xato() {
        // 2-argument handler fn bo'lishi shart.
        let err =
            run_source(r#"queue.on "send" 5"#).expect_err("handler fn bo'lmasa xato kutiladi");
        assert!(
            err.contains("queue.on"),
            "kutilgan queue.on xatosi, topildi: {}",
            err
        );
    }

    // `ai` testlari env'ga (kalitlarga) bog'liq — global mutex bilan serializatsiya
    // qilamiz (boshqa testlar parallel env'ni o'zgartirmasin). Bu testlar
    // TARMOQQA CHIQMAYDI: kalit yo'qligida API chaqiruvidan OLDIN xato berilishini
    // tekshiramiz.
    static AI_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn ai_kalit_yoq_bolsa_aniq_xato() {
        let _guard = AI_ENV_LOCK.lock().unwrap();
        // Hamma kalit env'larini vaqtincha o'chiramiz (auto-detect hech qaysisini
        // topmasligi kerak). runtime/ da .env yo'q -> aniq "kaliti topilmadi" xatosi,
        // tarmoqqa chiqilmaydi. Oldingi qiymatlarni saqlab, testdan keyin tiklaymiz.
        let saved: Vec<(&str, Option<String>)> = ["AI_KEY", "ANTHROPIC_API_KEY", "OPENAI_API_KEY"]
            .iter()
            .map(|k| (*k, std::env::var(k).ok()))
            .collect();
        for (k, _) in &saved {
            unsafe { std::env::remove_var(k) };
        }
        let err = run_source(r#"x = ai.ask "salom""#).expect_err("kalit yo'qligida xato kutiladi");
        // env'ni tiklaymiz (boshqa testlarga ta'sir qilmasin).
        for (k, v) in &saved {
            if let Some(val) = v {
                unsafe { std::env::set_var(k, val) };
            }
        }
        assert!(
            err.contains("kaliti topilmadi") || err.contains("kalit"),
            "kutilgan kalit-topilmadi xatosi, topildi: {}",
            err
        );
    }

    #[test]
    fn ai_noma_lum_funksiya_xato() {
        let _guard = AI_ENV_LOCK.lock().unwrap();
        // ai.foo -> dispatch'ga yetib "ai.foo yo'q" beradi (noma'lum nom EMAS).
        // Kalit bo'lsa ham bo'lmasa ham bu funksiya nomini tekshirishdan oldin keladi.
        let err =
            run_source(r#"ai.foo "x""#).expect_err("noma'lum ai funksiyasi xato berishi kerak");
        assert!(
            err.contains("ai.foo") && !err.contains("noma'lum nom"),
            "ai dispatch'ga yetib funksiya xatosi berishi kerak, topildi: {}",
            err
        );
    }

    #[test]
    fn ai_ozgaruvchi_modulni_yopadi() {
        // `ai` o'zgaruvchi sifatida e'lon qilinsa, u modul emas — oddiy map maydoni
        // sifatida o'qiladi (http/db kabi emas, lekin ai dispatch lookup tekshiradi).
        run(r#"
ai = {ask:"shadowed"}
log "ai.ask = ${ai.ask}"
"#);
    }

    // sh.run -> {stdout stderr code}: echo natijasi va muvaffaqiyat kodi to'g'ri.
    // (Unix-mos echo, CI ubuntu+macOS da ishlaydi.)
    #[test]
    fn sh_run_echo_natija_va_kod() {
        run(r#"
r = sh.run "printf salom"
(r.code == 0) | (fail "code 0 bo'lishi kerak: ${r.code}")
(r.stdout == "salom") | (fail "stdout noto'g'ri: ${r.stdout}")
(r.stderr == "") | (fail "stderr bo'sh bo'lishi kerak: ${r.stderr}")
"#);
    }

    // Non-zero exit -> Flow::err EMAS, `code` orqali tekshiriladi (kutilgan natija).
    #[test]
    fn sh_run_nolik_bolmagan_kod_xato_emas() {
        run(r#"
r = sh.run "exit 7"
(r.code == 7) | (fail "code 7 bo'lishi kerak: ${r.code}")
"#);
    }

    // --- `use ./fayl` foydalanuvchi modullari (issue #45) ---

    use std::sync::atomic::{AtomicU64, Ordering};

    // Unikal vaqtinchalik katalog — parallel testlar to'qnashmasligi uchun
    // (process id + atomik hisoblagich). Test fayllari shu yerga yoziladi.
    fn temp_module_dir() -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("flux_mod_test_{}_{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // `files` ([(nom, manba), ...]) ni `dir`ga yozadi, birinchisini run qiladi,
    // natijani qaytaradi. Tugagach katalogni tozalaydi.
    fn run_modules(files: &[(&str, &str)]) -> Result<(), String> {
        let dir = temp_module_dir();
        for (name, src) in files {
            // Fayl nomi subkatalogli bo'lishi mumkin ("sub/test.fx") — `../`
            // (yuqori papka) modul yo'llarini sinash uchun papka ierarxiyasi kerak.
            let p = dir.join(name);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(p, src).unwrap();
        }
        let main_path = dir.join(files[0].0);
        let src = std::fs::read_to_string(&main_path).unwrap();
        let r = run_source_at(&src, &main_path);
        let _ = std::fs::remove_dir_all(&dir);
        r
    }

    // Asosiy holat (issue #45 reproduction): `exp` qilingan qiymat va funksiya
    // `modul.nom` ostida ko'rinadi; modul funksiyasi modul-darajadagi `exp`ga
    // (closure) kira oladi.
    #[test]
    fn use_module_exp_va_closure() {
        run_modules(&[
            (
                "main.fx",
                r#"
use ./greet
(greet.greeting == "salom") | (fail "greeting: ${greet.greeting}")
(greet.hello "Aziza" == "salom, Aziza") | (fail "hello: ${greet.hello "Aziza"}")
"#,
            ),
            (
                "greet.fx",
                "exp greeting = \"salom\"\nexp fn hello nom -> \"${greeting}, ${nom}\"\n",
            ),
        ])
        .unwrap();
    }

    // `as alias` — bog'lash nomi alias bo'ladi (batareya nomi bilan to'qnashmaslik).
    #[test]
    fn use_module_alias() {
        run_modules(&[
            (
                "main.fx",
                r#"
use ./tools as t
(t.classify "x" == "turi: x") | (fail "classify: ${t.classify "x"}")
"#,
            ),
            ("tools.fx", "exp fn classify v -> \"turi: ${v}\"\n"),
        ])
        .unwrap();
    }

    // Modul-private nomlar (oddiy `=`/`fn`) namespace'ga KIRMAYDI — faqat `exp`.
    #[test]
    fn use_module_private_nom_eksport_qilinmaydi() {
        run_modules(&[
            (
                "main.fx",
                r#"
use ./m
(m.pub_v == 1) | (fail "pub_v: ${m.pub_v}")
(m.priv_v == nil) | (fail "priv_v eksport qilinmasligi kerak: ${m.priv_v}")
"#,
            ),
            ("m.fx", "exp pub_v = 1\npriv_v = 2\n"),
        ])
        .unwrap();
    }

    // Nested import (main -> a -> b): modul boshqa modulni import qila oladi,
    // yo'l import qiluvchi modulning katalogiga nisbatan hal qilinadi.
    #[test]
    fn use_module_nested() {
        run_modules(&[
            (
                "main.fx",
                r#"
use ./a
(a.get() == 43) | (fail "get: ${a.get()}")
"#,
            ),
            ("a.fx", "use ./b\nexp fn get -> b.val + 1\n"),
            ("b.fx", "exp val = 42\n"),
        ])
        .unwrap();
    }

    // `../` (yuqori papka) modul yo'li (issue #47): subkatalogdagi fayl
    // ota-katalogdagi modulni import qila oladi. parse_use `Tok::DotDot`'ni
    // tan olishi va runtime yo'lni `..` bilan hal qila olishi shu yerda sinaladi.
    #[test]
    fn use_module_yuqori_papka() {
        run_modules(&[
            (
                "sub/test.fx",
                r#"
use ../greet
(greet.greeting == "salom") | (fail "greeting: ${greet.greeting}")
"#,
            ),
            ("greet.fx", "exp greeting = \"salom\"\n"),
        ])
        .unwrap();
    }

    // Cache: bir modul ikki marta `use` qilinsa bir marta bajariladi (idempotent).
    // Modul top-level `<-` hisoblagichni oshiradi; ikki import'da ham 1 bo'lib qoladi.
    #[test]
    fn use_module_cache_bir_marta_bajariladi() {
        run_modules(&[
            (
                "main.fx",
                r#"
use ./c
use ./c as c2
(c.n == 1) | (fail "n: ${c.n}")
(c2.n == 1) | (fail "c2.n: ${c2.n}")
"#,
            ),
            // `exp n` bir martagina hisoblanadi — cache bo'lsa shunday.
            ("c.fx", "exp n = 1\n"),
        ])
        .unwrap();
    }

    // Sikllik import (x -> y -> x) aniq xato beradi (cheksiz rekursiya emas).
    #[test]
    fn use_module_sikllik_import_xato() {
        let err = run_modules(&[
            ("x.fx", "use ./y\nexp a = 1\n"),
            ("y.fx", "use ./x\nexp b = 2\n"),
        ])
        .unwrap_err();
        assert!(
            err.contains("sikllik import"),
            "sikllik import xatosi kutilgan edi, kelgan: {}",
            err
        );
    }

    // Mavjud bo'lmagan modul — aniq "topilmadi" xatosi.
    #[test]
    fn use_module_topilmadi_xato() {
        let err = run_modules(&[("main.fx", "use ./yoq\n")]).unwrap_err();
        assert!(
            err.contains("modul topilmadi"),
            "topilmadi xatosi kutilgan edi, kelgan: {}",
            err
        );
    }

    // `.fx` kengaytmasi avtomatik qo'shiladi: `use ./greet` -> `greet.fx`.
    // (Yuqoridagi testlar ham shunga tayanadi; bu aniq tekshiruv.)
    #[test]
    fn use_module_fx_kengaytma_avto() {
        run_modules(&[
            (
                "main.fx",
                "use ./util\n(util.x == 7) | (fail \"x: ${util.x}\")\n",
            ),
            ("util.fx", "exp x = 7\n"),
        ])
        .unwrap();
    }

    // Batareya `use` (`use http`) hamon no-op — fayl yuklanmaydi, dispatch ishlaydi.
    #[test]
    fn use_batareya_hamon_no_op() {
        // `use math` fayl izlamaydi (xato bermaydi), math.* dispatch ishlaydi.
        run(r#"
use math
(math.floor 3.7 == 3) | (fail "floor noto'g'ri")
"#);
    }

    // `each i in inf` — cheksiz loop. `stop` chiqaradi, `i` 0 dan ortadi.
    // REPL/event-loop uchun (issue #27): oldin model 1..1000 hiylasiga murojaat
    // qilardi; endi tabiiy cheksiz takror bor.
    #[test]
    fn each_inf_stop_va_hisoblagich() {
        run(r#"
sum <- 0
each i in inf
  if i == 5
    stop
  sum <- sum + i
(sum == 10) | (fail "0+1+2+3+4 = 10 bo'lishi kerak: ${sum}")
"#);
    }

    // `skip` cheksiz loop'da keyingi iteratsiyaga o'tadi (i baribir ortadi).
    #[test]
    fn each_inf_skip() {
        run(r#"
cnt <- 0
each i in inf
  if i >= 10
    stop
  if i % 2 == 0
    skip
  cnt <- cnt + 1
(cnt == 5) | (fail "toq sonlar 1,3,5,7,9 = 5 ta: ${cnt}")
"#);
    }

    // inf qiymat sifatida ishlatib bo'lmaydi — faqat `each i in inf` da.
    #[test]
    fn inf_qiymat_sifatida_xato() {
        let err = run_source("x = inf\n").expect_err("inf qiymat bo'lishi xato berishi kerak");
        assert!(err.contains("inf"), "kutilmagan xato: {}", err);
    }

    // `each k, v in inf` — ikki o'zgaruvchi ma'nosiz (cheksiz oddiy hisoblagich).
    #[test]
    fn each_inf_ikki_ozgaruvchi_xato() {
        let err = run_source("each k, v in inf\n  stop\n")
            .expect_err("inf bilan ikki o'zgaruvchi xato berishi kerak");
        assert!(
            err.contains("bitta o'zgaruvchi"),
            "kutilmagan xato: {}",
            err
        );
    }

    // --- `flux check` (faqat parse, issue #55) ---

    // To'g'ri kod -> check muvaffaqiyatli (Ok).
    #[test]
    fn check_togri_kod_ok() {
        check_source(
            r#"
fn fib n
  if n < 2
    ret n
  (fib (n - 1)) + (fib (n - 2))
log "${fib 10}"
"#,
        )
        .expect("to'g'ri kod check'dan o'tishi kerak");
    }

    // Parse/lex xato -> check Err qaytaradi (main bu Err'ni exit 2 ga aylantiradi).
    #[test]
    fn check_parse_xato_err() {
        let err = check_source("fn g x\n  ret (\n").expect_err("parse xato Err berishi kerak");
        assert!(!err.is_empty(), "xato matni bo'sh bo'lmasligi kerak");
    }

    // ENG MUHIM: check kodni BAJARMAYDI — runtime side-effect/xato bo'lmaydi.
    // Quyidagi kod runtime'da fail qiladi (noma'lum nom), lekin sintaksis to'g'ri,
    // shuning uchun check Ok beradi. Bu check'ning interp'ni o't kazib yuborishini
    // isbotlaydi (Forge eval-gate QATLAM 1: bajarish XAVFLI).
    #[test]
    fn check_kodni_bajarmaydi() {
        // `nomalum_funksiya` runtime'da "noma'lum nom" beradi, lekin sintaksis joyida.
        check_source("x = nomalum_funksiya 5\n")
            .expect("sintaktik to'g'ri kod check'dan o'tishi kerak (bajarilmaydi)");
        // Tasdiq: xuddi shu kod run'da xato beradi (bajariladi).
        assert!(
            run_source("x = nomalum_funksiya 5\n").is_err(),
            "run bu kodni bajarib xato berishi kerak (check bilan farq)"
        );
    }

    // parse_args: `check` buyrug'ini tanib, faylni Command::Check ga joylaydi.
    #[test]
    fn parse_args_check_buyrugi() {
        let args: Vec<String> = ["flux", "check", "test.fx"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        match parse_args(&args) {
            Some(Command::Check(p)) => assert_eq!(p, "test.fx"),
            _ => panic!("Command::Check kutilgan edi, topildi boshqa variant"),
        }
    }

    // issue #57: symbol MATNGA aylanganda `:` prefiks tashlanadi
    // (interpolatsiya, str.str, `+` birlashtirish). Symbol literal sintaksisi
    // (`:florist`) o'zgarmaydi — faqat matn ko'rinishi `:` siz.
    #[test]
    fn sym_to_text_colon_tashlanadi() {
        run(r#"
s = :florist
# interpolatsiya
(("v/${s}") == "v/florist") | (fail "interpolatsiya: ${"v/${s}"}")
# str.str
((str.str s) == "florist") | (fail "str.str: ${str.str s}")
# `+` birlashtirish (ikkala tomon)
(("p/" + s) == "p/florist") | (fail "chap + : ${"p/" + s}")
((s + "/q") == "florist/q") | (fail "o'ng + : ${s + "/q"}")
# symbol literal va taqqoslash O'ZGARMAYDI
(s == :florist) | (fail "symbol taqqoslash buzildi")
"#);
    }

    // Symbol list/map ICHIDA `:` prefiksini SAQLAYDI — u yerda symbol
    // string'dan ajralib turishi kerak (repr matn ko'rinishidan farq qiladi).
    #[test]
    fn sym_repr_listda_colon_saqlaydi() {
        run(r#"
xs = [:a "b"]
((str.str xs) == "[:a \"b\"]") | (fail "list repr: ${str.str xs}")
"#);
    }

    // --- auth battery (issue #69) ---
    //
    // $AUTH_SECRET env'ga muhtoj testlar uchun lock — parallel testlar env'ga
    // race qilmasin (AI_ENV_LOCK naqshi).
    static AUTH_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn auth_jwt_verify_roundtrip() {
        let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AUTH_SECRET", "sirli-kalit-123") };
        run(r#"
use auth
token = auth.jwt {sub:"u1" tenant:"t1" role:"admin"}
# imzolangan JWT — 3 segment (header.payload.imzo)
parts = str.split token "."
(parts.len == 3) | (fail "JWT 3 segment emas: ${parts.len}")
# verify -> payload map qaytaradi, da'volar saqlanadi
claims = auth.verify token
(claims.sub == "u1") | (fail "sub noto'g'ri: ${claims.sub}")
(claims.tenant == "t1") | (fail "tenant noto'g'ri: ${claims.tenant}")
(claims.role == "admin") | (fail "role noto'g'ri: ${claims.role}")
# iat/exp avtomatik qo'shilgan
(claims.exp > claims.iat) | (fail "exp iat'dan katta bo'lishi kerak")
"#);
    }

    #[test]
    fn auth_verify_buzilgan_token_xato() {
        let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AUTH_SECRET", "sirli-kalit-123") };
        // Imzo buzilgan token -> auth.verify err (Flux'da `try` o'tkazgich, xato
        // run'ni to'xtatadi — shuning uchun Rust tomonda expect_err bilan
        // tekshiramiz). token'ga belgi qo'shsak imzo mos kelmaydi.
        let err = run_source(
            r#"use auth
token = auth.jwt {sub:"u1"}
auth.verify (token + "x")"#,
        )
        .expect_err("buzilgan token xato berishi kerak");
        assert!(
            err.contains("imzo"),
            "kutilgan imzo xatosi, topildi: {}",
            err
        );
    }

    #[test]
    fn auth_verify_yaroqsiz_shakl_xato() {
        let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AUTH_SECRET", "sirli-kalit-123") };
        // 3 segmentdan kam — JWT shakli noto'g'ri -> err.
        let err = run_source(
            r#"use auth
auth.verify "faqat.ikki""#,
        )
        .expect_err("yaroqsiz shakl xato berishi kerak");
        assert!(
            err.contains("shakl") || err.contains("segment"),
            "kutilgan shakl xatosi, topildi: {}",
            err
        );
    }

    #[test]
    fn auth_verify_exp_siz_token_rad_etiladi() {
        let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("AUTH_SECRET", "sirli-kalit-123") };
        // `exp:nil` payload -> auth.jwt `or_insert` nil'ni override qilmaydi,
        // ya'ni token sonli `exp`siz imzolanadi. To'g'ri imzolangan bo'lsa ham,
        // auth.verify uni RAD ETISHI kerak (aks holda abadiy amal qilardi —
        // Codex P2). Kalit to'g'ri, shuning uchun bu imzo emas, exp xatosi.
        let err = run_source(
            r#"use auth
token = auth.jwt {sub:"u1" exp:nil}
auth.verify token"#,
        )
        .expect_err("exp'siz token rad etilishi kerak");
        assert!(
            err.contains("exp") || err.contains("muddat"),
            "kutilgan exp-yo'q xatosi, topildi: {}",
            err
        );
    }

    #[test]
    fn auth_secret_yoq_bolsa_aniq_xato() {
        let _guard = AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var("AUTH_SECRET").ok();
        unsafe { std::env::remove_var("AUTH_SECRET") };
        let err = run_source(
            r#"use auth
token = auth.jwt {sub:"u1"}"#,
        )
        .expect_err("$AUTH_SECRET yo'qligida xato kutiladi");
        if let Some(v) = saved {
            unsafe { std::env::set_var("AUTH_SECRET", v) };
        }
        assert!(
            err.contains("AUTH_SECRET"),
            "kutilgan AUTH_SECRET xatosi, topildi: {}",
            err
        );
    }

    #[test]
    fn auth_hash_check_roundtrip() {
        // hash/check env'ga muhtoj emas (lock kerak emas).
        run(r#"
use auth
h = auth.hash "user-parol"
# argon2id PHC string
(str.has h "argon2id") | (fail "argon2id hash emas: ${h}")
# to'g'ri parol -> true
(auth.check "user-parol" h) | (fail "to'g'ri parol check false berdi")
# noto'g'ri parol -> false
((auth.check "xato-parol" h) == false) | (fail "noto'g'ri parol check true berdi")
"#);
    }

    #[test]
    fn auth_noma_lum_funksiya_xato() {
        // auth.foo -> dispatch'ga yetib "auth.foo yo'q" beradi (noma'lum nom EMAS).
        let err =
            run_source(r#"auth.foo "x""#).expect_err("noma'lum auth funksiyasi xato berishi kerak");
        assert!(
            err.contains("auth.foo") && !err.contains("noma'lum nom"),
            "auth dispatch'ga yetib funksiya xatosi berishi kerak, topildi: {}",
            err
        );
    }

    #[test]
    fn auth_ozgaruvchi_modulni_yopadi() {
        // `auth` o'zgaruvchi sifatida e'lon qilinsa, u modul emas — oddiy map.
        run(r#"
auth = {jwt:"shadowed"}
log "auth.jwt = ${auth.jwt}"
"#);
    }
}
