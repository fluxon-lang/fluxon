// Flux runtime — buyruq qatori interfeysi.
//
// Foydalanish:
//   flux run <fayl.fx>     — Flux faylini bajaradi
//   flux <fayl.fx>         — xuddi shu (qisqartma)

// mimalloc — parallel'da system malloc'dan ancha kam contention beradi.
// Interpreter qisqa umrli scope allokatsiyalarini ko'p qiladi (tree-walking).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod ai_mod;
mod ast;
mod builtins;
mod cron_mod;
mod db_mod;
mod http_mod;
mod interp;
mod lexer;
mod parser;
mod queue_mod;
mod reg_mod;
mod token;
mod value;
mod ws_mod;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let path = match parse_args(&args) {
        Some(p) => p,
        None => {
            eprintln!("Foydalanish: flux run <fayl.fx>");
            return ExitCode::from(2);
        }
    };

    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Faylni o'qib bo'lmadi '{}': {}", path, e);
            return ExitCode::from(1);
        }
    };

    match run_source(&src) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Flux xato: {}", e);
            ExitCode::from(1)
        }
    }
}

fn parse_args(args: &[String]) -> Option<String> {
    match args.get(1).map(|s| s.as_str()) {
        Some("run") => args.get(2).cloned(),
        Some(p) if !p.starts_with('-') => Some(p.to_string()),
        _ => None,
    }
}

fn run_source(src: &str) -> Result<(), String> {
    let toks = lexer::lex(src)?;
    let prog = parser::parse(toks)?;
    // Arc<Interp>: http.serve handler'larni server thread'larida apply qiladi,
    // shuning uchun interp thread'lar orasida ulashiladigan bo'lishi kerak.
    let interp = interp::Interp::new_arc();
    interp.run(&prog)
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

    #[test]
    fn map_operations() {
        run(r#"
u = {name:"Aziza" age:30}
u2 = u.set "age" 31
log "keys=${u.keys} hasName=${u.has "name"} age=${u2.age}"
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
}
