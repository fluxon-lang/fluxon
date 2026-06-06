// Flux yadro kutubxonasi (batteries'siz qism).
//
// Uch xil chaqiruv:
//   - global funksiyalar (log) — Env'ga o'rnatiladi (`install`)
//   - modul funksiyalari (str.up, math.floor, rand.int, json) — `call_module`
//   - qiymat metodlari (l.push, m.set, s.up emas...) — `call_method`
//
// list metodlari qiymat ustida (.push/.filter), str/math/rand modul orqali —
// bu spec'dagi farqni aniq aks ettiradi: `l.len` (a'zo) vs `str.len s` (modul).

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::interp::{Env, Flow};
use crate::value::{NativeFn, Value};

type R = Result<Value, Flow>;

// --- global funksiyalarni o'rnatish ---
pub fn install(env: &Env) {
    let mut s = env.write();
    let mut add = |name: &str, f: Box<dyn Fn(Vec<Value>) -> R + Send + Sync>| {
        s.set_global(
            name,
            Value::Native(Arc::new(NativeFn {
                name: name.into(),
                func: f,
            })),
        );
    };
    add(
        "log",
        Box::new(|args: Vec<Value>| {
            let parts: Vec<String> = args.iter().map(|v| format!("{}", v)).collect();
            eprintln!("{}", parts.join(" "));
            Ok(Value::Nil)
        }),
    );
    // rep status body — HTTP javobi. Yangi Value variant qo'shmaslik uchun
    // maxsus kalitli map sifatida ifodalanadi: {__resp:true status:N body:V}.
    // http_mod::value_to_response shu kalitni taniydi.
    add(
        "rep",
        Box::new(|args: Vec<Value>| {
            let status = match args.first() {
                Some(Value::Int(n)) => *n,
                Some(other) => {
                    return Err(Flow::err(format!(
                        "rep: 1-argument status (int) bo'lishi kerak, {} berildi",
                        other.type_name()
                    )));
                }
                None => return Err(Flow::err("rep: status argumenti kerak")),
            };
            let body = args.get(1).cloned().unwrap_or(Value::Nil);
            let mut m = BTreeMap::new();
            m.insert("__resp".to_string(), Value::Bool(true));
            m.insert("status".to_string(), Value::Int(status));
            m.insert("body".to_string(), body);
            Ok(Value::Map(m))
        }),
    );
}

// --- modul nomimi? ---
pub fn is_module(name: &str) -> bool {
    matches!(
        name,
        "str" | "math" | "rand" | "json" | "time" | "io" | "fs" | "sh"
    )
}

// --- modul funksiyasi chaqiruvi ---
pub fn call_module(module: &str, func: &str, args: Vec<Value>) -> R {
    match module {
        "str" => str_module(func, args),
        "math" => math_module(func, args),
        "rand" => rand_module(func, args),
        "json" => json_module(func, args),
        "time" => time_module(func, args),
        "io" => io_module(func, args),
        "fs" => fs_module(func, args),
        "sh" => sh_module(func, args),
        _ => Err(Flow::err(format!("noma'lum modul: {}", module))),
    }
}

// ---------------- str ----------------
fn str_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "len" => {
            let s = arg_str(&args, 0, "str.len")?;
            Ok(Value::Int(s.chars().count() as i64))
        }
        "up" => Ok(Value::Str(arg_str(&args, 0, "str.up")?.to_uppercase())),
        "low" => Ok(Value::Str(arg_str(&args, 0, "str.low")?.to_lowercase())),
        "slice" => {
            let s = arg_str(&args, 0, "str.slice")?;
            let a = arg_int(&args, 1, "str.slice")? as usize;
            let b = arg_int(&args, 2, "str.slice")? as usize;
            let chars: Vec<char> = s.chars().collect();
            let a = a.min(chars.len());
            let b = b.min(chars.len());
            if a >= b {
                return Ok(Value::Str(String::new()));
            }
            Ok(Value::Str(chars[a..b].iter().collect()))
        }
        "split" => {
            let s = arg_str(&args, 0, "str.split")?;
            let sep = arg_str(&args, 1, "str.split")?;
            let parts: Vec<Value> = if sep.is_empty() {
                s.chars().map(|c| Value::Str(c.to_string())).collect()
            } else {
                s.split(&sep).map(|p| Value::Str(p.to_string())).collect()
            };
            Ok(Value::List(parts))
        }
        "has" => {
            let s = arg_str(&args, 0, "str.has")?;
            let sub = arg_str(&args, 1, "str.has")?;
            Ok(Value::Bool(s.contains(&sub)))
        }
        "int" => {
            let s = arg_str(&args, 0, "str.int")?;
            match s.trim().parse::<i64>() {
                Ok(n) => Ok(Value::Int(n)),
                Err(_) => Ok(Value::Nil),
            }
        }
        "str" => Ok(Value::Str(format!("{}", arg(&args, 0, "str.str")?))),
        _ => Err(Flow::err(format!(
            "str modulida '{}' funksiyasi yo'q",
            func
        ))),
    }
}

// ---------------- math ----------------
fn math_module(func: &str, args: Vec<Value>) -> R {
    let x = arg_num(&args, 0, &format!("math.{}", func))?;
    match func {
        "floor" => Ok(Value::Int(x.floor() as i64)),
        "ceil" => Ok(Value::Int(x.ceil() as i64)),
        "abs" => {
            // int kirsa int, flt kirsa flt qaytaramiz
            match &args[0] {
                Value::Int(n) => Ok(Value::Int(n.abs())),
                _ => Ok(Value::Flt(x.abs())),
            }
        }
        "round" => Ok(Value::Int(x.round() as i64)),
        _ => Err(Flow::err(format!(
            "math modulida '{}' funksiyasi yo'q",
            func
        ))),
    }
}

// ---------------- rand (dependency'siz LCG) ----------------
fn rand_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "int" => {
            let a = arg_int(&args, 0, "rand.int")?;
            let b = arg_int(&args, 1, "rand.int")?;
            if b < a {
                return Err(Flow::err("rand.int: yuqori chegara pastdan kichik"));
            }
            let span = (b - a + 1) as u64;
            let r = next_rand() % span;
            Ok(Value::Int(a + r as i64))
        }
        "str" => {
            let n = arg_int(&args, 0, "rand.str")? as usize;
            const ALPHA: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
            let mut out = String::with_capacity(n);
            for _ in 0..n {
                let idx = (next_rand() % ALPHA.len() as u64) as usize;
                out.push(ALPHA[idx] as char);
            }
            Ok(Value::Str(out))
        }
        _ => Err(Flow::err(format!(
            "rand modulida '{}' funksiyasi yo'q",
            func
        ))),
    }
}

// ---------------- time ----------------
// Barcha vaqtlar UTC matn "YYYY-MM-DD HH:MM:SS" formatida — SQLite
// CURRENT_TIMESTAMP (tbl `now` ustuni) bilan AYNAN bir xil, shuning uchun
// `created > (time.ago 24 :hr)` kabi DB filtrlari to'g'ridan-to'g'ri ishlaydi.
fn time_module(func: &str, args: Vec<Value>) -> R {
    match func {
        // hozirgi vaqt -> UTC matn timestamp
        "now" => Ok(Value::Str(fmt_unix(now_unix()))),
        // time.ago N :birlik -> hozirdan N birlik oldingi UTC matn
        "ago" => {
            let n = arg_int(&args, 0, "time.ago")?;
            let unit = arg_str(&args, 1, "time.ago")?;
            let secs = unit_secs(&unit).ok_or_else(|| {
                Flow::err(format!(
                    "time.ago: birlik :sec/:min/:hr/:day bo'lishi kerak, :{} berildi",
                    unit
                ))
            })?;
            Ok(Value::Str(fmt_unix(now_unix() - n * secs)))
        }
        // time.fmt timestamp "..." -> matn formatlash.
        // Kirish: matn timestamp ("YYYY-MM-DD HH:MM:SS") yoki unix int.
        // Token'lar: YYYY MM DD HH mm ss
        "fmt" => {
            let ts = match arg(&args, 0, "time.fmt")? {
                Value::Str(s) => parse_ts(s).ok_or_else(|| {
                    Flow::err(format!("time.fmt: timestamp matnini o'qib bo'lmadi: {}", s))
                })?,
                Value::Int(n) => *n,
                other => {
                    return Err(Flow::err(format!(
                        "time.fmt: 1-argument timestamp (str/int) bo'lishi kerak, {} berildi",
                        other.type_name()
                    )));
                }
            };
            let pat = arg_str(&args, 1, "time.fmt")?;
            Ok(Value::Str(strftime(ts, &pat)))
        }
        _ => Err(Flow::err(format!(
            "time modulida '{}' funksiyasi yo'q",
            func
        ))),
    }
}

// ---------------- io ----------------
// Terminal input/output. `log` har doim stderr'ga `\n` qo'shadi; interaktiv CLI
// (REPL, agent, wizard) uchun stdin'dan o'qish va `\n`siz prompt kerak. Prompt
// va kiritma stdout/stdin orqali ketadi (log stderr — ular aralashmasin).
fn io_module(func: &str, args: Vec<Value>) -> R {
    use std::io::Write;
    match func {
        // io.read_line -> stdin'dan bitta satr (oxirgi \n olib tashlanadi).
        // EOF (Ctrl-D, quvur tugashi) -> nil, shunda chaqiruvchi tsiklni to'xtatadi.
        "read_line" => {
            let mut line = String::new();
            match std::io::stdin().read_line(&mut line) {
                Ok(0) => Ok(Value::Nil),
                Ok(_) => {
                    // satr oxiridagi \n (va Windows \r) ni olib tashlaymiz
                    let trimmed = line.trim_end_matches(['\n', '\r']);
                    Ok(Value::Str(trimmed.to_string()))
                }
                Err(e) => Err(Flow::err(format!("io.read_line: {}", e))),
            }
        }
        // io.print s -> stdout'ga \n SIZ chiqarish (prompt ko'rsatish uchun).
        // Darhol flush — aks holda prompt buferda qolib, foydalanuvchi kiritmadan
        // oldin uni ko'rmaydi.
        "print" => {
            let s = arg_str(&args, 0, "io.print")?;
            let mut out = std::io::stdout();
            out.write_all(s.as_bytes())
                .and_then(|_| out.flush())
                .map_err(|e| Flow::err(format!("io.print: {}", e)))?;
            Ok(Value::Nil)
        }
        // io.prompt msg -> msg'ni \n siz chiqarib, keyin bitta satr o'qiydi.
        // io.print + io.read_line uchun qulay shorthand.
        "prompt" => {
            let s = arg_str(&args, 0, "io.prompt")?;
            let mut out = std::io::stdout();
            out.write_all(s.as_bytes())
                .and_then(|_| out.flush())
                .map_err(|e| Flow::err(format!("io.prompt: {}", e)))?;
            io_module("read_line", vec![])
        }
        _ => Err(Flow::err(format!("io modulida '{}' funksiyasi yo'q", func))),
    }
}

// ---------------- fs (lokal fayl tizimi) ----------------
//
// Konvensiya: muvaffaqiyatda foydali qiymat (matn/bool/ro'yxat) yoki :ok sym;
// haqiqiy IO xatosida Flow::err — sababni yo'qotmaslik uchun (io battery shunday).
// Yagona istisno: fs.read fayl yo'qligida nil qaytaradi (bu kutilgan holat, xato
// emas — "fayl bormi?" tekshiruvini read ichida birlashtirish uchun qulay).
fn fs_module(func: &str, args: Vec<Value>) -> R {
    match func {
        // fs.read path -> fayl matni (str), yoki fayl yo'q bo'lsa nil.
        // UTF-8 emas faylda yoki ruxsat xatosida Flow::err.
        "read" => {
            let path = arg_str(&args, 0, "fs.read")?;
            match std::fs::read_to_string(&path) {
                Ok(s) => Ok(Value::Str(s)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Nil),
                Err(e) => Err(Flow::err(format!("fs.read {}: {}", path, e))),
            }
        }
        // fs.write path content -> faylni ustiga yozadi (oldingi mazmun o'chadi).
        // Oraliq papkalar mavjud bo'lishi kerak (kerak bo'lsa fs.mkdirp).
        "write" => {
            let path = arg_str(&args, 0, "fs.write")?;
            let content = arg_str(&args, 1, "fs.write")?;
            std::fs::write(&path, content)
                .map_err(|e| Flow::err(format!("fs.write {}: {}", path, e)))?;
            Ok(Value::Sym("ok".into()))
        }
        // fs.append path content -> mavjud fayl oxiriga qo'shadi (yo'q bo'lsa yaratadi).
        "append" => {
            use std::io::Write;
            let path = arg_str(&args, 0, "fs.append")?;
            let content = arg_str(&args, 1, "fs.append")?;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| Flow::err(format!("fs.append {}: {}", path, e)))?;
            f.write_all(content.as_bytes())
                .map_err(|e| Flow::err(format!("fs.append {}: {}", path, e)))?;
            Ok(Value::Sym("ok".into()))
        }
        // fs.exists path -> bool (fayl YOKI papka mavjudmi).
        "exists" => {
            let path = arg_str(&args, 0, "fs.exists")?;
            Ok(Value::Bool(std::path::Path::new(&path).exists()))
        }
        // fs.ls path -> papka ichidagi nomlar ro'yxati [str] (to'liq yo'l emas,
        // faqat nom). Tartib deterministik bo'lishi uchun saralanadi.
        "ls" => {
            let path = arg_str(&args, 0, "fs.ls")?;
            let entries = std::fs::read_dir(&path)
                .map_err(|e| Flow::err(format!("fs.ls {}: {}", path, e)))?;
            let mut names = Vec::new();
            for entry in entries {
                let entry = entry.map_err(|e| Flow::err(format!("fs.ls {}: {}", path, e)))?;
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
            names.sort();
            Ok(Value::List(names.into_iter().map(Value::Str).collect()))
        }
        // fs.del path -> faylni yoki bo'sh papkani o'chiradi -> :ok.
        // Papka bo'sh bo'lmasa Flow::err (rekursiv o'chirish ataylab yo'q —
        // tasodifiy butun daraxtni o'chirib qo'ymaslik uchun xavfsizroq).
        "del" => {
            let path = arg_str(&args, 0, "fs.del")?;
            let p = std::path::Path::new(&path);
            let res = if p.is_dir() {
                std::fs::remove_dir(p)
            } else {
                std::fs::remove_file(p)
            };
            res.map_err(|e| Flow::err(format!("fs.del {}: {}", path, e)))?;
            Ok(Value::Sym("ok".into()))
        }
        // fs.mkdirp path -> papkani (kerakli oraliq papkalar bilan) yaratadi -> :ok.
        // Papka allaqachon mavjud bo'lsa xato emas (idempotent).
        "mkdirp" => {
            let path = arg_str(&args, 0, "fs.mkdirp")?;
            std::fs::create_dir_all(&path)
                .map_err(|e| Flow::err(format!("fs.mkdirp {}: {}", path, e)))?;
            Ok(Value::Sym("ok".into()))
        }
        _ => Err(Flow::err(format!("fs modulida '{}' funksiyasi yo'q", func))),
    }
}

// ---------------- sh (tashqi shell buyruqlari) ----------------
//
// sh.run cmd -> {stdout: str  stderr: str  code: int}.
// Buyruq SHELL orqali ishga tushiriladi (Unix: `sh -c`, Windows: `cmd /C`) —
// shunda `cd x && cargo build`, quvurlar (`|`), `&&`, glob kabi shell xususiyatlari
// ishlaydi (issue #26 da Sonnet aynan shu naqshni taxmin qildi). Bu coding agent,
// CI skript, build avtomatizatsiyasi uchun kerak.
//
// `code == 0` muvaffaqiyat (Unix konvensiyasi). Jarayon signal bilan o'lsa (Unix'da
// exit code yo'q) code = -1. Buyruqning O'ZI muvaffaqiyatsiz bo'lishi (non-zero code)
// Flow::err EMAS — bu kutilgan natija, chaqiruvchi `code` orqali tekshiradi. Faqat
// jarayonni umuman boshlab bo'lmasa (masalan shell topilmasa) Flow::err.
//
// Xavfli buyruqlarni bloklash ataylab YO'Q — bu foydalanuvchi mas'uliyati (issue #26).
fn sh_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "run" => {
            let cmd = arg_str(&args, 0, "sh.run")?;
            let mut command;
            #[cfg(windows)]
            {
                command = std::process::Command::new("cmd");
                command.arg("/C").arg(&cmd);
            }
            #[cfg(not(windows))]
            {
                command = std::process::Command::new("sh");
                command.arg("-c").arg(&cmd);
            }
            let output = command
                .output()
                .map_err(|e| Flow::err(format!("sh.run: buyruqni boshlab bo'lmadi: {}", e)))?;
            // stdout/stderr ni lossy UTF-8 sifatida o'qiymiz — ikkilik chiqishda ham
            // panic bo'lmaydi (json dekoderdan farqli, bu yerda matn kafolati yo'q).
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            // signal bilan tugagan jarayonda kod None bo'ladi -> -1.
            let code = output.status.code().unwrap_or(-1) as i64;
            let mut m = BTreeMap::new();
            m.insert("stdout".to_string(), Value::Str(stdout));
            m.insert("stderr".to_string(), Value::Str(stderr));
            m.insert("code".to_string(), Value::Int(code));
            Ok(Value::Map(m))
        }
        _ => Err(Flow::err(format!("sh modulida '{}' funksiyasi yo'q", func))),
    }
}

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn unit_secs(unit: &str) -> Option<i64> {
    match unit {
        "sec" => Some(1),
        "min" => Some(60),
        "hr" => Some(3600),
        "day" => Some(86_400),
        _ => None,
    }
}

// unix sekund -> (year, month, day, hour, min, sec) UTC.
// civil_from_days: Howard Hinnant algoritmi (dependency'siz, doimiy vaqt).
fn civil(unix: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = unix.div_euclid(86_400);
    let secs_of_day = unix.rem_euclid(86_400);
    let (hh, mm, ss) = (
        (secs_of_day / 3600) as u32,
        ((secs_of_day % 3600) / 60) as u32,
        (secs_of_day % 60) as u32,
    );
    // days: 1970-01-01 dan boshlab. Hinnant: era'ni mart'dan boshlaydi.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11] (mart=0)
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, hh, mm, ss)
}

fn fmt_unix(unix: i64) -> String {
    let (y, mo, d, h, mi, s) = civil(unix);
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, mo, d, h, mi, s)
}

// "YYYY-MM-DD HH:MM:SS" (yoki "YYYY-MM-DDTHH:MM:SS") -> unix sekund (UTC).
fn parse_ts(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> { s.get(a..z)?.parse::<i64>().ok() };
    let y = num(0, 4)?;
    let mo = num(5, 7)?;
    let d = num(8, 10)?;
    let h = num(11, 13)?;
    let mi = num(14, 16)?;
    let se = num(17, 19)?;
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se)
}

// (year, month, day) UTC -> 1970-01-01 dan kunlar (Hinnant teskari).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 }; // mart=0
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn strftime(unix: i64, pat: &str) -> String {
    let (y, mo, d, h, mi, s) = civil(unix);
    pat.replace("YYYY", &format!("{:04}", y))
        .replace("MM", &format!("{:02}", mo))
        .replace("DD", &format!("{:02}", d))
        .replace("HH", &format!("{:02}", h))
        .replace("mm", &format!("{:02}", mi))
        .replace("ss", &format!("{:02}", s))
}

// Oddiy xorshift RNG. Seed system time'dan bir marta olinadi.
fn next_rand() -> u64 {
    use std::cell::Cell;
    thread_local! {
        static STATE: Cell<u64> = Cell::new(seed());
    }
    STATE.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        x
    })
}

fn seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E3779B97F4A7C15);
    nanos | 1 // nol bo'lmasligi uchun
}

// ---------------- json ----------------
fn json_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "enc" => Ok(Value::Str(json_encode(arg(&args, 0, "json.enc")?))),
        "dec" => {
            let s = arg_str(&args, 0, "json.dec")?;
            json_decode(&s)
        }
        _ => Err(Flow::err(format!(
            "json modulida '{}' funksiyasi yo'q",
            func
        ))),
    }
}

pub fn json_encode(v: &Value) -> String {
    match v {
        Value::Int(n) => n.to_string(),
        Value::Flt(x) => x.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Nil => "null".into(),
        Value::Str(s) | Value::Sym(s) => json_str(s),
        Value::List(items) => {
            let parts: Vec<String> = items.iter().map(json_encode).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Map(m) => {
            let parts: Vec<String> = m
                .iter()
                .map(|(k, val)| format!("{}:{}", json_str(k), json_encode(val)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Fn(_) | Value::Native(_) => "null".into(),
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

// Minimal JSON dekoder (yadro versiyasi uchun yetarli).
pub fn json_decode(s: &str) -> R {
    let mut p = JsonParser {
        b: s.as_bytes(),
        i: 0,
    };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    Ok(v)
}

struct JsonParser<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> JsonParser<'a> {
    fn skip_ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }
    fn value(&mut self) -> R {
        self.skip_ws();
        if self.i >= self.b.len() {
            return Err(Flow::err("json: kutilmagan oxir"));
        }
        match self.b[self.i] {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => Ok(Value::Str(self.string()?)),
            b't' | b'f' => self.boolean(),
            b'n' => {
                self.i += 4;
                Ok(Value::Nil)
            }
            _ => self.number(),
        }
    }
    fn object(&mut self) -> R {
        self.i += 1; // {
        let mut m = BTreeMap::new();
        self.skip_ws();
        if self.i < self.b.len() && self.b[self.i] == b'}' {
            self.i += 1;
            return Ok(Value::Map(m));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            if self.i >= self.b.len() || self.b[self.i] != b':' {
                return Err(Flow::err("json: ':' kutilgan"));
            }
            self.i += 1;
            let val = self.value()?;
            m.insert(key, val);
            self.skip_ws();
            match self.b.get(self.i) {
                Some(b',') => {
                    self.i += 1;
                }
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(Flow::err("json: ',' yoki '}' kutilgan")),
            }
        }
        Ok(Value::Map(m))
    }
    fn array(&mut self) -> R {
        self.i += 1; // [
        let mut out = Vec::new();
        self.skip_ws();
        if self.i < self.b.len() && self.b[self.i] == b']' {
            self.i += 1;
            return Ok(Value::List(out));
        }
        loop {
            let v = self.value()?;
            out.push(v);
            self.skip_ws();
            match self.b.get(self.i) {
                Some(b',') => {
                    self.i += 1;
                }
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(Flow::err("json: ',' yoki ']' kutilgan")),
            }
        }
        Ok(Value::List(out))
    }
    fn string(&mut self) -> Result<String, Flow> {
        if self.b[self.i] != b'"' {
            return Err(Flow::err("json: satr kutilgan"));
        }
        self.i += 1;
        // Natijani BAYTLAR sifatida yig'amiz, oxirida UTF-8 str'ga aylantiramiz —
        // ko'p baytli belgilar (emoji, o'zbekcha o'/g') bayt-bayt `as char` bilan
        // BUZILADI (mojibake). \u escape'lari esa char'ga dekodlanib UTF-8 yoziladi.
        let mut out: Vec<u8> = Vec::new();
        while self.i < self.b.len() {
            let c = self.b[self.i];
            self.i += 1;
            match c {
                b'"' => {
                    return String::from_utf8(out)
                        .map_err(|_| Flow::err("json: satr noto'g'ri UTF-8"));
                }
                b'\\' => {
                    let e = self.b[self.i];
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0C),
                        b'u' => {
                            // \uXXXX — 16-bitli kod birligi. Surrogate juftligi
                            // (\uD800..DBFF + \uDC00..DFFF) bitta belgini beradi
                            // (emoji va BMP'dan tashqari hamma narsa shunday keladi).
                            let hi = self.hex4()?;
                            let ch = if (0xD800..=0xDBFF).contains(&hi) {
                                // yuqori surrogate -> past surrogatni kutamiz.
                                if self.b.get(self.i) == Some(&b'\\')
                                    && self.b.get(self.i + 1) == Some(&b'u')
                                {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    let cp = 0x10000
                                        + (((hi as u32 - 0xD800) << 10) | (lo as u32 - 0xDC00));
                                    char::from_u32(cp).unwrap_or('\u{FFFD}')
                                } else {
                                    '\u{FFFD}' // juftsiz surrogate
                                }
                            } else {
                                char::from_u32(hi as u32).unwrap_or('\u{FFFD}')
                            };
                            // char'ni UTF-8 baytlar sifatida qo'shamiz.
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => out.push(other), // noma'lum escape — baytni o'zini
                    }
                }
                // Oddiy bayt (ASCII yoki ko'p baytli UTF-8 ketma-ketligining bir
                // qismi) — o'z holicha qo'shiladi, str konversiyasi oxirida.
                _ => out.push(c),
            }
        }
        Err(Flow::err("json: yopilmagan satr"))
    }

    // Joriy pozitsiyadan 4 hex raqamni o'qib u16 qaytaradi (\uXXXX uchun).
    fn hex4(&mut self) -> Result<u16, Flow> {
        if self.i + 4 > self.b.len() {
            return Err(Flow::err("json: \\u uchun 4 hex raqam kerak"));
        }
        let slice = &self.b[self.i..self.i + 4];
        let s = std::str::from_utf8(slice).map_err(|_| Flow::err("json: \\u noto'g'ri"))?;
        let v = u16::from_str_radix(s, 16).map_err(|_| Flow::err("json: \\u noto'g'ri hex"))?;
        self.i += 4;
        Ok(v)
    }
    fn boolean(&mut self) -> R {
        if self.b[self.i..].starts_with(b"true") {
            self.i += 4;
            Ok(Value::Bool(true))
        } else if self.b[self.i..].starts_with(b"false") {
            self.i += 5;
            Ok(Value::Bool(false))
        } else {
            Err(Flow::err("json: noto'g'ri bool"))
        }
    }
    fn number(&mut self) -> R {
        let start = self.i;
        let mut is_float = false;
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c.is_ascii_digit() || c == b'-' || c == b'+' {
                self.i += 1;
            } else if c == b'.' || c == b'e' || c == b'E' {
                is_float = true;
                self.i += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).unwrap_or("");
        if is_float {
            text.parse::<f64>()
                .map(Value::Flt)
                .map_err(|_| Flow::err("json: noto'g'ri son"))
        } else {
            text.parse::<i64>()
                .map(Value::Int)
                .map_err(|_| Flow::err("json: noto'g'ri son"))
        }
    }
}

// ---------------- qiymat metodlari (list/map) ----------------
pub fn call_method(recv: &Value, method: &str, args: Vec<Value>) -> R {
    match recv {
        Value::List(xs) => list_method(xs, method, args),
        Value::Map(m) => map_method(m, method, args),
        Value::Str(_) => Err(Flow::err(format!(
            "str metodlari modul orqali chaqiriladi: str.{} s (qiymat metodi emas)",
            method
        ))),
        other => Err(Flow::err(format!(
            "{} tipida '.{}' metodi yo'q",
            other.type_name(),
            method
        ))),
    }
}

fn list_method(xs: &[Value], method: &str, args: Vec<Value>) -> R {
    match method {
        "len" => Ok(Value::Int(xs.len() as i64)),
        "push" => {
            let mut new = xs.to_vec();
            new.push(arg(&args, 0, "list.push")?.clone());
            Ok(Value::List(new))
        }
        "has" => {
            let needle = arg(&args, 0, "list.has")?;
            Ok(Value::Bool(xs.iter().any(|v| v.equals(needle))))
        }
        "join" => {
            let sep = arg_str(&args, 0, "list.join")?;
            let parts: Vec<String> = xs.iter().map(|v| format!("{}", v)).collect();
            Ok(Value::Str(parts.join(&sep)))
        }
        "slice" => {
            let a = arg_int(&args, 0, "list.slice")? as usize;
            let b = arg_int(&args, 1, "list.slice")? as usize;
            let a = a.min(xs.len());
            let b = b.min(xs.len());
            if a >= b {
                return Ok(Value::List(vec![]));
            }
            Ok(Value::List(xs[a..b].to_vec()))
        }
        // filter/map/reduce — funksiya argument oladi; interp uni shu yerda
        // chaqira olmaydi (apply Interp'da). Shuning uchun bu metodlar maxsus
        // ishlov talab qiladi — pastdagi izohga qarang.
        "filter" | "map" | "reduce" => Err(Flow::err(format!(
            "ichki: list.{} alohida yo'l bilan ishlov beriladi",
            method
        ))),
        _ => Err(Flow::err(format!("list metodi '{}' mavjud emas", method))),
    }
}

fn map_method(m: &BTreeMap<String, Value>, method: &str, args: Vec<Value>) -> R {
    match method {
        "len" => Ok(Value::Int(m.len() as i64)),
        "has" => {
            let k = key_of(arg(&args, 0, "map.has")?);
            Ok(Value::Bool(m.contains_key(&k)))
        }
        "keys" => Ok(Value::List(
            m.keys().map(|k| Value::Str(k.clone())).collect(),
        )),
        "vals" => Ok(Value::List(m.values().cloned().collect())),
        "set" => {
            let k = key_of(arg(&args, 0, "map.set")?);
            let v = arg(&args, 1, "map.set")?.clone();
            let mut new = m.clone();
            new.insert(k, v);
            Ok(Value::Map(new))
        }
        "del" => {
            let k = key_of(arg(&args, 0, "map.del")?);
            let mut new = m.clone();
            new.remove(&k);
            Ok(Value::Map(new))
        }
        _ => Err(Flow::err(format!("map metodi '{}' mavjud emas", method))),
    }
}

fn key_of(v: &Value) -> String {
    match v {
        Value::Str(s) | Value::Sym(s) => s.clone(),
        other => format!("{}", other),
    }
}

// ---------------- argument yordamchilari ----------------
fn arg<'a>(args: &'a [Value], i: usize, who: &str) -> Result<&'a Value, Flow> {
    args.get(i)
        .ok_or_else(|| Flow::err(format!("{}: {}-argument yetishmadi", who, i + 1)))
}
fn arg_str(args: &[Value], i: usize, who: &str) -> Result<String, Flow> {
    match arg(args, i, who)? {
        Value::Str(s) => Ok(s.clone()),
        Value::Sym(s) => Ok(s.clone()),
        other => Err(Flow::err(format!(
            "{}: {}-argument str bo'lishi kerak, {} berildi",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}
fn arg_int(args: &[Value], i: usize, who: &str) -> Result<i64, Flow> {
    match arg(args, i, who)? {
        Value::Int(n) => Ok(*n),
        other => Err(Flow::err(format!(
            "{}: {}-argument int bo'lishi kerak, {} berildi",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}
fn arg_num(args: &[Value], i: usize, who: &str) -> Result<f64, Flow> {
    match arg(args, i, who)? {
        Value::Int(n) => Ok(*n as f64),
        Value::Flt(x) => Ok(*x),
        other => Err(Flow::err(format!(
            "{}: {}-argument son bo'lishi kerak, {} berildi",
            who,
            i + 1,
            other.type_name()
        ))),
    }
}

#[cfg(test)]
mod time_tests {
    use super::*;

    // Ma'lum unix nuqtalar (UTC) — chrono'siz civil algoritmini tekshiramiz.
    #[test]
    fn civil_known_points() {
        assert_eq!(fmt_unix(0), "1970-01-01 00:00:00"); // epoch
        assert_eq!(fmt_unix(1_700_000_000), "2023-11-14 22:13:20");
        // 2024-02-29 (kabisa yili) — 12:00:00 UTC
        assert_eq!(fmt_unix(1_709_208_000), "2024-02-29 12:00:00");
    }

    #[test]
    fn parse_then_fmt_roundtrip() {
        for &u in &[0i64, 1_700_000_000, 1_709_208_000, 4_102_444_800] {
            let s = fmt_unix(u);
            assert_eq!(parse_ts(&s), Some(u), "round-trip buzildi: {}", s);
        }
        // 'T' ajratuvchi ham qo'llab-quvvatlanadi (ISO).
        assert_eq!(parse_ts("2023-11-14T22:13:20"), Some(1_700_000_000));
    }

    #[test]
    fn ago_subtracts_units() {
        let now = now_unix();
        // 24 soat = 1 kun: ikki yo'l bir xil natija (matn).
        assert_eq!(fmt_unix(now - 24 * 3600), fmt_unix(now - 86_400));
    }

    #[test]
    fn parse_rejects_garbage() {
        assert_eq!(parse_ts("salom"), None);
        assert_eq!(parse_ts("2023-11-14"), None); // juda qisqa (vaqt yo'q)
    }
}

#[cfg(test)]
mod io_tests {
    use super::*;

    // io.print qiymat sifatida nil qaytaradi (stdout'ga yozish — yon ta'sir).
    // Test stdout'ga "" yozadi (bo'sh) — kuzatuvchi chiqishni ifloslamaydi.
    // (Value/Flow Debug derive qilmaydi — unwrap o'rniga match.)
    #[test]
    fn print_returns_nil() {
        match io_module("print", vec![Value::Str(String::new())]) {
            Ok(Value::Nil) => {}
            _ => panic!("io.print nil qaytarishi kerak"),
        }
    }

    // io.print argument str bo'lishini talab qiladi.
    #[test]
    fn print_requires_str_arg() {
        assert!(io_module("print", vec![Value::Int(5)]).is_err());
        assert!(io_module("print", vec![]).is_err());
    }

    // Noma'lum io funksiyasi aniq xato beradi. (Flow Debug derive qilmaydi —
    // xato matniga Flow::Error ichidan kiramiz.)
    #[test]
    fn unknown_func_errors() {
        match io_module("yoq", vec![]) {
            Err(Flow::Error(msg)) => assert!(msg.contains("io modulida")),
            _ => panic!("Flow::Error kutilgan edi"),
        }
    }

    // io modul sifatida tanilishi kerak (argumentsiz Field dispatch shunга tayanadi).
    #[test]
    fn io_is_module() {
        assert!(is_module("io"));
    }
}

#[cfg(test)]
mod fs_tests {
    use super::*;

    // Har test uchun noyob vaqtinchalik papka (boshqa testlar bilan to'qnashmasin).
    // Process pid + test nomi yetarli noyob — testlar parallel ishlasa ham.
    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("flux_fs_test_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&p); // oldingi qoldiqni tozalash
        std::fs::create_dir_all(&p).expect("tmp dir yaratilmadi");
        p
    }

    fn path_str(dir: &std::path::Path, name: &str) -> String {
        dir.join(name).to_string_lossy().into_owned()
    }

    // write + read aylanasi: yozilgan matn aynan o'qiladi.
    #[test]
    fn write_then_read() {
        let dir = tmp_dir("write_read");
        let f = path_str(&dir, "a.txt");
        match fs_module(
            "write",
            vec![Value::Str(f.clone()), Value::Str("salom".into())],
        ) {
            Ok(Value::Sym(s)) if s == "ok" => {}
            _ => panic!("fs.write :ok qaytarishi kerak"),
        }
        match fs_module("read", vec![Value::Str(f)]) {
            Ok(Value::Str(s)) => assert_eq!(s, "salom"),
            _ => panic!("fs.read yozilgan matnni qaytarishi kerak"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Yo'q faylni o'qish nil qaytaradi (xato emas) — issue talabi.
    #[test]
    fn read_missing_is_nil() {
        let dir = tmp_dir("read_missing");
        let f = path_str(&dir, "yoq.txt");
        match fs_module("read", vec![Value::Str(f)]) {
            Ok(Value::Nil) => {}
            _ => panic!("yo'q fayl nil qaytarishi kerak"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // append bo'lmagan faylni yaratadi va ketma-ket qo'shadi.
    #[test]
    fn append_accumulates() {
        let dir = tmp_dir("append");
        let f = path_str(&dir, "log.txt");
        let _ = fs_module(
            "append",
            vec![Value::Str(f.clone()), Value::Str("a".into())],
        );
        let _ = fs_module(
            "append",
            vec![Value::Str(f.clone()), Value::Str("b".into())],
        );
        match fs_module("read", vec![Value::Str(f)]) {
            Ok(Value::Str(s)) => assert_eq!(s, "ab"),
            _ => panic!("append matnni to'plashi kerak"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // exists: mavjud fayl true, yo'q fayl false.
    #[test]
    fn exists_reflects_reality() {
        let dir = tmp_dir("exists");
        let f = path_str(&dir, "bor.txt");
        let _ = fs_module("write", vec![Value::Str(f.clone()), Value::Str("x".into())]);
        match fs_module("exists", vec![Value::Str(f)]) {
            Ok(Value::Bool(true)) => {}
            _ => panic!("mavjud fayl true bo'lishi kerak"),
        }
        match fs_module("exists", vec![Value::Str(path_str(&dir, "yoq.txt"))]) {
            Ok(Value::Bool(false)) => {}
            _ => panic!("yo'q fayl false bo'lishi kerak"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ls: papka ichidagi nomlarni saralangan holda qaytaradi.
    #[test]
    fn ls_lists_sorted_names() {
        let dir = tmp_dir("ls");
        let _ = fs_module(
            "write",
            vec![Value::Str(path_str(&dir, "b.txt")), Value::Str("".into())],
        );
        let _ = fs_module(
            "write",
            vec![Value::Str(path_str(&dir, "a.txt")), Value::Str("".into())],
        );
        match fs_module("ls", vec![Value::Str(dir.to_string_lossy().into_owned())]) {
            Ok(Value::List(items)) => {
                let names: Vec<String> = items
                    .iter()
                    .map(|v| match v {
                        Value::Str(s) => s.clone(),
                        _ => panic!("ls str ro'yxati qaytarishi kerak"),
                    })
                    .collect();
                assert_eq!(names, vec!["a.txt".to_string(), "b.txt".to_string()]);
            }
            _ => panic!("ls ro'yxat qaytarishi kerak"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // del: faylni o'chiradi, keyin exists false bo'ladi.
    #[test]
    fn del_removes_file() {
        let dir = tmp_dir("del");
        let f = path_str(&dir, "o.txt");
        let _ = fs_module("write", vec![Value::Str(f.clone()), Value::Str("x".into())]);
        match fs_module("del", vec![Value::Str(f.clone())]) {
            Ok(Value::Sym(s)) if s == "ok" => {}
            _ => panic!("fs.del :ok qaytarishi kerak"),
        }
        match fs_module("exists", vec![Value::Str(f)]) {
            Ok(Value::Bool(false)) => {}
            _ => panic!("o'chirilgan fayl mavjud bo'lmasligi kerak"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // mkdirp: rekursiv papka yaratadi va idempotent (ikkinchi marta ham :ok).
    #[test]
    fn mkdirp_recursive_and_idempotent() {
        let dir = tmp_dir("mkdirp");
        let nested = path_str(&dir, "a/b/c");
        match fs_module("mkdirp", vec![Value::Str(nested.clone())]) {
            Ok(Value::Sym(s)) if s == "ok" => {}
            _ => panic!("fs.mkdirp :ok qaytarishi kerak"),
        }
        assert!(std::path::Path::new(&nested).is_dir());
        // ikkinchi chaqiruv xato bermasligi kerak (idempotent)
        match fs_module("mkdirp", vec![Value::Str(nested)]) {
            Ok(Value::Sym(s)) if s == "ok" => {}
            _ => panic!("mkdirp idempotent bo'lishi kerak"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Noma'lum fs funksiyasi aniq xato beradi.
    #[test]
    fn unknown_func_errors() {
        match fs_module("yoq", vec![]) {
            Err(Flow::Error(msg)) => assert!(msg.contains("fs modulida")),
            _ => panic!("Flow::Error kutilgan edi"),
        }
    }

    // fs modul sifatida tanilishi kerak.
    #[test]
    fn fs_is_module() {
        assert!(is_module("fs"));
    }
}

#[cfg(test)]
mod sh_tests {
    use super::*;

    // Buyruq turlarini matn sifatida olish (xato matnlarini soddalashtirish uchun).
    fn run(cmd: &str) -> BTreeMap<String, Value> {
        match sh_module("run", vec![Value::Str(cmd.into())]) {
            Ok(Value::Map(m)) => m,
            other => panic!("sh.run map qaytarishi kerak, {:?} keldi", other.is_ok()),
        }
    }

    // Oddiy echo: stdout to'g'ri, code 0, stderr bo'sh.
    #[test]
    fn echo_stdout_and_code() {
        let m = run("echo salom");
        match m.get("stdout") {
            Some(Value::Str(s)) => assert_eq!(s.trim_end(), "salom"),
            _ => panic!("stdout str bo'lishi kerak"),
        }
        assert!(matches!(m.get("code"), Some(Value::Int(0))));
        match m.get("stderr") {
            Some(Value::Str(s)) => assert!(s.is_empty()),
            _ => panic!("stderr str bo'lishi kerak"),
        }
    }

    // Non-zero exit: buyruq muvaffaqiyatsiz -> Flow::err EMAS, code != 0.
    #[test]
    fn nonzero_exit_is_not_error() {
        let m = run("exit 3");
        assert!(matches!(m.get("code"), Some(Value::Int(3))));
    }

    // stderr alohida tutiladi (stdout bilan aralashmaydi).
    #[test]
    fn stderr_captured_separately() {
        let m = run("echo xato 1>&2");
        match m.get("stderr") {
            Some(Value::Str(s)) => assert_eq!(s.trim_end(), "xato"),
            _ => panic!("stderr str bo'lishi kerak"),
        }
        match m.get("stdout") {
            Some(Value::Str(s)) => assert!(s.is_empty()),
            _ => panic!("stdout str bo'lishi kerak"),
        }
    }

    // Shell xususiyatlari (`&&`, quvur) ishlaydi — buyruq shell orqali boradi.
    #[test]
    fn shell_features_work() {
        let m = run("echo bir && echo ikki");
        match m.get("stdout") {
            Some(Value::Str(s)) => {
                assert!(s.contains("bir") && s.contains("ikki"));
            }
            _ => panic!("stdout str bo'lishi kerak"),
        }
        assert!(matches!(m.get("code"), Some(Value::Int(0))));
    }

    // Noma'lum sh funksiyasi aniq xato beradi.
    #[test]
    fn unknown_func_errors() {
        match sh_module("yoq", vec![]) {
            Err(Flow::Error(msg)) => assert!(msg.contains("sh modulida")),
            _ => panic!("Flow::Error kutilgan edi"),
        }
    }

    // sh modul sifatida tanilishi kerak.
    #[test]
    fn sh_is_module() {
        assert!(is_module("sh"));
    }
}
