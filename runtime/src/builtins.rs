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
            let parts: Vec<String> = args.iter().map(|v| v.to_text()).collect();
            eprintln!("{}", parts.join(" "));
            Ok(Value::Nil)
        }),
    );
    // rep status body [headers] — HTTP javobi. Yangi Value variant qo'shmaslik
    // uchun maxsus kalitli map sifatida ifodalanadi:
    // {__resp:true status:N body:V headers:{...}}. http_mod::value_to_response
    // shu kalitlarni taniydi.
    //
    // Ixtiyoriy 3-argument — custom header'lar map'i (issue #16). Body'dan
    // alohida 3-arg qilingani body bilan to'qnashmaslik uchun: `rep 200 {ok}`
    // da butun map = body, shuning uchun header'ni body ichidan o'qib bo'lmaydi.
    // Header qiymati str (bitta sarlavha) yoki list (takror sarlavha, masalan
    // bir nechta Set-Cookie) bo'lishi mumkin.
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
            // 3-argument bo'lsa — headers map. Map bo'lmasa aniq xato beramiz,
            // chunki jim e'tiborsiz qoldirish AI uchun chalg'ituvchi.
            if let Some(h) = args.get(2) {
                match h {
                    Value::Map(_) => {
                        m.insert("headers".to_string(), h.clone());
                    }
                    other => {
                        return Err(Flow::err(format!(
                            "rep: 3-argument headers (map) bo'lishi kerak, {} berildi",
                            other.type_name()
                        )));
                    }
                }
            }
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
        "str" => Ok(Value::Str(arg(&args, 0, "str.str")?.to_text())),
        // str.sym "pending" → :pending. Query-string statuslarini sym filtrga
        // aylantirish uchun (db.eq {status:(str.split q "," |> ...).map str.sym}).
        // Avval bu uchun json.dec("\":"+s+"\"") hack ishlatilardi (issue #78).
        // Sym/str ham qabul qilinadi (idempotent); atrofdagi bo'sh joy kesiladi.
        "sym" => match arg(&args, 0, "str.sym")? {
            Value::Str(s) => Ok(Value::Sym(s.trim().to_string())),
            Value::Sym(s) => Ok(Value::Sym(s.clone())),
            other => Err(Flow::err(format!(
                "str.sym: str kutilgan, {} berildi",
                other.type_name()
            ))),
        },
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
            // int kirsa int, flt kirsa flt qaytaramiz.
            // i64::MIN.abs() panic beradi (musbat juftligi sig'maydi) — checked.
            match &args[0] {
                Value::Int(n) => Ok(Value::Int(
                    n.checked_abs().ok_or_else(|| Flow::overflow("math.abs"))?,
                )),
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
            // span i128 da: a/b chekka qiymatlarda (masalan a juda manfiy,
            // b juda musbat) b - a + 1 i64 ga sig'maydi va overflow berardi.
            let span = (b as i128) - (a as i128) + 1; // [1, 2^64]
            let r = if span > u64::MAX as i128 {
                next_rand() // to'liq i64 oralig'i — har qanday u64 mos qiymat
            } else {
                next_rand() % (span as u64)
            };
            // Haqiqiy natija a + r doim [a, b] ichida — ikkilik to'ldiruvchi
            // modulyar arifmetikasida wrapping_add aynan shu qiymatni beradi
            // (oraliq yig'indi i64 dan toshsa ham).
            Ok(Value::Int(a.wrapping_add(r as i64)))
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
            // Katta N da n * secs (yoki ayirma) i64 dan toshadi — checked.
            let ts = n
                .checked_mul(secs)
                .and_then(|off| now_unix().checked_sub(off))
                .ok_or_else(|| Flow::overflow("time.ago"))?;
            Ok(Value::Str(fmt_unix(ts)))
        }
        // time.in N :birlik -> hozirdan N birlik KEYINGI UTC matn (TTL/expiry).
        // time.ago ning ko'zgusi — yagona farq qo'shish/ayirish ishorasi.
        "in" => {
            let n = arg_int(&args, 0, "time.in")?;
            let unit = arg_str(&args, 1, "time.in")?;
            let secs = unit_secs(&unit).ok_or_else(|| {
                Flow::err(format!(
                    "time.in: birlik :sec/:min/:hr/:day bo'lishi kerak, :{} berildi",
                    unit
                ))
            })?;
            let ts = n
                .checked_mul(secs)
                .and_then(|off| now_unix().checked_add(off))
                .ok_or_else(|| Flow::overflow("time.in"))?;
            Ok(Value::Str(fmt_unix(ts)))
        }
        // time.sleep secs -> secs soniya kutadi (flt ham — 0.5 yarim soniya).
        // Polling/retry backoff uchun: xato holatda qayta urinishdan oldin
        // kutish (burst/rate-limit halqasini oldini olish). Manfiy qiymat 0 ga
        // klamp qilinadi (Duration::from_secs_f64 manfiyda panic beradi).
        "sleep" => {
            let secs = arg_num(&args, 0, "time.sleep")?.max(0.0);
            std::thread::sleep(std::time::Duration::from_secs_f64(secs));
            Ok(Value::Nil)
        }
        // time.fmt timestamp "..." -> matn formatlash.
        // Kirish: matn timestamp ("YYYY-MM-DD HH:MM:SS", ISO mintaqa ham) yoki unix int.
        // Token'lar: YYYY MM DD HH mm ss. Sukutda UTC wall-clock'ni formatlaydi.
        //
        // Ixtiyoriy 3-argument — IANA zona nomi: `time.fmt t "HH:mm" "Asia/Tashkent"`.
        // UTC instant'ni o'sha zonaning local wall-clock'iga (DST hisobga olinib)
        // o'tkazib formatlaydi — foydalanuvchiga mahalliy vaqtni ko'rsatish uchun.
        "fmt" => {
            let ts = arg_ts(&args, 0, "time.fmt")?;
            let pat = arg_str(&args, 1, "time.fmt")?;
            match args.get(2) {
                Some(_) => {
                    let zone = arg_str(&args, 2, "time.fmt")?;
                    let out = fmt_in_zone(ts, &pat, &zone).ok_or_else(|| {
                        Flow::err(format!("time.fmt: noma'lum IANA zona nomi: {}", zone))
                    })?;
                    Ok(Value::Str(out))
                }
                None => Ok(Value::Str(strftime(ts, &pat))),
            }
        }
        // time.parse "2026-06-10T10:00:00Z" -> kanonik UTC matn timestamp.
        // Ixtiyoriy ISO-8601 matnni (mijoz/tashqi API bergan) ichki kanonik
        // "YYYY-MM-DD HH:MM:SS" UTC formatiga keltiradi — shunda time.add/time.diff
        // va DB filtrlari u bilan bevosita ishlaydi. "Z", "±HH:MM"/"±HHMM" mintaqa
        // va kasr sekundni tushunadi; mintaqasiz matn UTC deb qabul qilinadi.
        //
        // Ixtiyoriy 2-argument — IANA zona nomi: `time.parse "2026-03-08 09:00" "America/New_York"`.
        // Bu holda matndagi wall-clock vaqt o'sha zonada (DST hisobga olinib) talqin
        // qilinadi va UTC ga aylantiriladi — fiksrlangan offset emas. "09:00 local"
        // har kuni to'g'ri UTC ga tushadi, yoz/qish o'tishida ham (PRD §6.8).
        "parse" => {
            let s = arg_str(&args, 0, "time.parse")?;
            let ts = match args.get(1) {
                Some(_) => {
                    let zone = arg_str(&args, 1, "time.parse")?;
                    parse_in_zone(&s, &zone).ok_or_else(|| {
                        Flow::err(format!(
                            "time.parse: '{}' zonasida '{}' vaqtini o'qib bo'lmadi \
                             (noma'lum zona yoki DST sakrashida mavjud bo'lmagan local vaqt)",
                            zone, s
                        ))
                    })?
                }
                None => parse_iso(&s).ok_or_else(|| {
                    Flow::err(format!(
                        "time.parse: ISO timestamp matnini o'qib bo'lmadi: {}",
                        s
                    ))
                })?,
            };
            Ok(Value::Str(fmt_unix(ts)))
        }
        // time.add t N :birlik -> t timestamp'ga N birlik QO'SHIB UTC matn qaytaradi.
        // time.in dan farqi: hozirdan emas, IXTIYORIY berilgan vaqtdan offset hisoblaydi
        // (masalan end_at = start_at + duration). N manfiy bo'lsa ayiradi (orqaga siljitadi).
        "add" => {
            let base = arg_ts(&args, 0, "time.add")?;
            let n = arg_int(&args, 1, "time.add")?;
            let unit = arg_str(&args, 2, "time.add")?;
            let secs = unit_secs(&unit).ok_or_else(|| {
                Flow::err(format!(
                    "time.add: birlik :sec/:min/:hr/:day bo'lishi kerak, :{} berildi",
                    unit
                ))
            })?;
            let ts = n
                .checked_mul(secs)
                .and_then(|off| base.checked_add(off))
                .ok_or_else(|| Flow::overflow("time.add"))?;
            Ok(Value::Str(fmt_unix(ts)))
        }
        // time.sub t N :birlik -> t timestamp'dan N birlik AYIRIB UTC matn qaytaradi.
        // time.add ning ko'zgusi (time.ago/time.in juftligi kabi). Qavssiz chaqiruvda
        // manfiy son binar `-` bilan adashishini oldini olish uchun alohida funksiya —
        // buffer-inclusive interval boshi `time.sub start_at 5 :min` deb yoziladi.
        "sub" => {
            let base = arg_ts(&args, 0, "time.sub")?;
            let n = arg_int(&args, 1, "time.sub")?;
            let unit = arg_str(&args, 2, "time.sub")?;
            let secs = unit_secs(&unit).ok_or_else(|| {
                Flow::err(format!(
                    "time.sub: birlik :sec/:min/:hr/:day bo'lishi kerak, :{} berildi",
                    unit
                ))
            })?;
            let ts = n
                .checked_mul(secs)
                .and_then(|off| base.checked_sub(off))
                .ok_or_else(|| Flow::overflow("time.sub"))?;
            Ok(Value::Str(fmt_unix(ts)))
        }
        // time.diff a b -> (a - b) ikki vaqt orasidagi farq SEKUNDDA (int).
        // Musbat natija = a, b dan keyin (kelajakda). Birlikka bo'lib o'tiladi
        // (masalan `(time.diff end start) / 60` -> daqiqada davomiylik).
        "diff" => {
            let a = arg_ts(&args, 0, "time.diff")?;
            let b = arg_ts(&args, 1, "time.diff")?;
            Ok(Value::Int(a - b))
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
    // Diapazonlarni tekshiramiz — days_from_civil overflow'ni jimgina
    // "tuzatadi" (mavjud bo'lmagan 02-31 -> 03-03), shuning uchun bu yerda
    // rad etamiz: booking oqimida noto'g'ri sana sukutsiz qabul qilinmasin.
    // se 60 — kabisa sekund (ISO ruxsat beradi) — qabul qilamiz.
    if !(1..=12).contains(&mo)
        || !(1..=days_in_month(y, mo)).contains(&d)
        || !(0..=23).contains(&h)
        || !(0..=59).contains(&mi)
        || !(0..=60).contains(&se)
    {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se)
}

// Berilgan yil/oy uchun kunlar soni (kabisa yilni hisobga oladi).
fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
            if leap { 29 } else { 28 }
        }
        _ => 0, // noto'g'ri oy — chaqiruvchi mo'ni allaqachon tekshiradi
    }
}

// Ixtiyoriy ISO-8601 matnni unix sekundga (UTC) o'giradi. parse_ts ustiga
// quriladi: avval sana+vaqt asosini ("YYYY-MM-DD[ T]HH:MM:SS") o'qiydi, so'ng
// 19-belgidan keyingi qismdan ixtiyoriy kasr sekund (".sss" — sekund aniqligida
// tashlanadi) va vaqt mintaqasini ("Z", "±HH:MM", "±HHMM", "±HH") tushunadi.
// Mintaqa ko'rsatilmasa UTC deb olinadi. Matndagi vaqt mahalliy -> UTC = vaqt - offset.
// Timestamp'lar ASCII, shuning uchun bayt indeksi = belgi indeksi (boundary xavfsiz).
fn parse_iso(s: &str) -> Option<i64> {
    let s = s.trim();
    let base = parse_ts(s)?; // birinchi 19 belgi (sana + vaqt); len >= 19 kafolat
    let mut rest = &s[19..];
    // kasr sekundni o'tkazib yuboramiz (".123") — sekund aniqligida ishlaymiz.
    if let Some(after_dot) = rest.strip_prefix('.') {
        let digits = after_dot.bytes().take_while(|b| b.is_ascii_digit()).count();
        rest = &after_dot[digits..];
    }
    let offset = match rest.chars().next() {
        None => 0,                  // mintaqasiz -> UTC
        Some('Z') | Some('z') => 0, // Zulu (UTC)
        Some(sign @ ('+' | '-')) => {
            // ":" ni e'tiborsiz qoldirib faqat raqamlarni olamiz: HHMM yoki HH.
            let digits: String = rest[1..].chars().filter(|c| c.is_ascii_digit()).collect();
            let (hh, mm) = match digits.len() {
                2 => (digits.parse::<i64>().ok()?, 0),
                4 => (
                    digits[0..2].parse::<i64>().ok()?,
                    digits[2..4].parse::<i64>().ok()?,
                ),
                _ => return None,
            };
            let off = hh * 3600 + mm * 60;
            if sign == '-' { -off } else { off }
        }
        _ => return None, // tanish bo'lmagan qoldiq -> noto'g'ri matn
    };
    Some(base - offset)
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
    strftime_fields(y, mo, d, h, mi, s, pat)
}

// Sana/vaqt maydonlaridan matn yasaydi — UTC (civil) va zona-aware (fmt_in_zone)
// yo'llari bir xil token to'plamini ishlatsin uchun ajratib olingan.
fn strftime_fields(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32, pat: &str) -> String {
    pat.replace("YYYY", &format!("{:04}", y))
        .replace("MM", &format!("{:02}", mo))
        .replace("DD", &format!("{:02}", d))
        .replace("HH", &format!("{:02}", h))
        .replace("mm", &format!("{:02}", mi))
        .replace("ss", &format!("{:02}", s))
}

// Wall-clock matnni IANA zonada (DST hisobga olinib) talqin qilib UTC sekundga
// o'giradi. parse_ts asosini (sana+vaqt, mintaqasiz) o'qiydi, so'ng o'sha maydonlarni
// zonaning local vaqti deb hisoblaydi — fiksrlangan offset emas, shuning uchun
// yoz/qish (DST) o'tishi to'g'ri ishlaydi.
//
// DST chetlari: bahorgi sakrashda mavjud bo'lmagan local vaqt (masalan 02:30) -> None
// (chaqiruvchi xato beradi). Kuzgi takror (vaqt ikki marta) holatda ertaroq (DST-li)
// instant tanlanadi — booking uchun deterministik va xavfsiz default.
fn parse_in_zone(s: &str, zone: &str) -> Option<i64> {
    use chrono::offset::LocalResult;
    use chrono::{NaiveDate, TimeZone};
    let tz: chrono_tz::Tz = zone.parse().ok()?;
    // parse_ts wall-clock'ni "soxta UTC" sekund sifatida beradi; civil bilan
    // maydonlarga qaytarib, zonada qayta talqin qilamiz.
    let (y, mo, d, h, mi, se) = civil(parse_ts(s)?);
    let naive = NaiveDate::from_ymd_opt(y as i32, mo, d)?.and_hms_opt(h, mi, se)?;
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt.timestamp()),
        LocalResult::Ambiguous(earlier, _later) => Some(earlier.timestamp()),
        LocalResult::None => None,
    }
}

// UTC instant'ni IANA zonaning local wall-clock'iga (DST hisobga olinib) o'tkazib
// formatlaydi. Noma'lum zona nomida None.
fn fmt_in_zone(unix: i64, pat: &str, zone: &str) -> Option<String> {
    use chrono::{Datelike, TimeZone, Timelike, Utc};
    let tz: chrono_tz::Tz = zone.parse().ok()?;
    let dt = Utc.timestamp_opt(unix, 0).single()?.with_timezone(&tz);
    Some(strftime_fields(
        dt.year() as i64,
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
        pat,
    ))
}

// OS kriptografik CSPRNG (getrandom orqali, `auth` battery'dagi OsRng bilan
// bir xil manba). Avval thread-local xorshift edi, seed = system time (nanos) —
// seed bashorat qilinardi va bir nanosekundda ochilgan ikki thread bir xil
// ketma-ketlik olardi. `rand.str` token/session-ID generatsiyaga tabiiy
// ishlatilgani uchun (#97) rand butunlay kriptografik xavfsiz manbaga o'tdi:
// bir ish = bir yo'l — alohida "xavfsiz rand" o'rganishga hojat yo'q.
fn next_rand() -> u64 {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    OsRng.next_u64()
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

// Map'ni JSON obyektga kodlaydi (Map va Ctx uchun umumiy).
fn json_encode_map(m: &std::collections::BTreeMap<String, Value>) -> String {
    let parts: Vec<String> = m
        .iter()
        .map(|(k, val)| format!("{}:{}", json_str(k), json_encode(val)))
        .collect();
    format!("{{{}}}", parts.join(","))
}

pub fn json_encode(v: &Value) -> String {
    match v {
        Value::Int(n) => n.to_string(),
        // JSON'da Infinity/NaN yo'q — JSON.stringify kabi `null` chiqaramiz
        // (aks holda "inf"/"NaN" qat'iy parserlarda rad etiladi).
        Value::Flt(x) => {
            if x.is_finite() {
                x.to_string()
            } else {
                "null".into()
            }
        }
        Value::Bool(b) => b.to_string(),
        Value::Nil => "null".into(),
        Value::Str(s) | Value::Sym(s) => json_str(s),
        Value::List(items) => {
            let parts: Vec<String> = items.iter().map(json_encode).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Map(m) => json_encode_map(m),
        // ctx oddiy map kabi kodlanadi (snapshot) — javob body'siga tushsa.
        Value::Ctx(c) => json_encode_map(&c.lock().unwrap()),
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
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            // Qolgan control belgilar (0x00–0x1F) JSON spec'da xom kelolmaydi —
            // \u00XX shaklida escape qilamiz (aks holda chiqish invalid JSON).
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
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
    // Qiymatdan keyin chiqindi qolmasligi kerak — `"1 garbage"` endi xato beradi
    // (avval jim `1` qaytarardi).
    if p.i < p.b.len() {
        return Err(Flow::err("json: qiymatdan keyin ortiqcha matn"));
    }
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
            // `null`ni harf-baharf tekshiramiz — avval `nqqq` ham jim nil berardi.
            b'n' => {
                if self.b[self.i..].starts_with(b"null") {
                    self.i += 4;
                    Ok(Value::Nil)
                } else {
                    Err(Flow::err("json: noto'g'ri qiymat (null kutilgan)"))
                }
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
        // Kesilgan input (masalan `{`) bilan chegaradan o'tib panic bo'lmasin —
        // ishonchsiz tashqi ma'lumot (HTTP body) crash qildirmasligi shart.
        if self.i >= self.b.len() {
            return Err(Flow::err("json: kutilmagan tugash"));
        }
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
                    // Satr `\` bilan tugab qolsa (masalan `"ab\`) escape baytini
                    // o'qishda chegaradan o'tmaylik — aks holda panic.
                    if self.i >= self.b.len() {
                        return Err(Flow::err("json: kutilmagan tugash"));
                    }
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
    // JSON son grammatikasini qat'iy tutamiz: [-] int [frac] [exp].
    // Avvalgi versiya `+5`, `1.2.3` kabi yaroqsiz sonlarni yutardi.
    fn number(&mut self) -> R {
        let start = self.i;
        let mut is_float = false;
        // ixtiyoriy manfiy belgi — JSON faqat '-' ruxsat beradi ('+' emas)
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        // butun qism: '0' yoki 1-9 dan boshlanuvchi raqamlar
        match self.b.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(c) if c.is_ascii_digit() => {
                while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
            }
            _ => return Err(Flow::err("json: noto'g'ri son")),
        }
        // kasr qismi: '.' dan keyin kamida bitta raqam
        if self.b.get(self.i) == Some(&b'.') {
            is_float = true;
            self.i += 1;
            if !self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                return Err(Flow::err("json: noto'g'ri son"));
            }
            while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                self.i += 1;
            }
        }
        // eksponent: e/E [+/-] kamida bitta raqam
        if matches!(self.b.get(self.i), Some(b'e') | Some(b'E')) {
            is_float = true;
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            if !self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                return Err(Flow::err("json: noto'g'ri son"));
            }
            while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                self.i += 1;
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
        "index" => {
            // Birinchi mos elementning indeksi; topilmasa -1 (bool'dan farqli,
            // index pozitsiya beradi — list.has bilan juftlik).
            let needle = arg(&args, 0, "list.index")?;
            let i = xs
                .iter()
                .position(|v| v.equals(needle))
                .map(|i| i as i64)
                .unwrap_or(-1);
            Ok(Value::Int(i))
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
        // Argumentsiz sort — tabiiy tartib (son/matn). Komparatorli varianti
        // funksiya argument olgani uchun interp'dagi list_hof orqali keladi.
        "sort" => sort_default(xs),
        "reverse" => {
            let mut new = xs.to_vec();
            new.reverse();
            Ok(Value::List(new))
        }
        "uniq" => {
            // Birinchi uchragan nusxa qoladi (tartib saqlanadi). Value hash'siz,
            // shuning uchun equals bilan chiziqli qidiruv — list'lar kichik.
            let mut out: Vec<Value> = Vec::new();
            for x in xs {
                if !out.iter().any(|v| v.equals(x)) {
                    out.push(x.clone());
                }
            }
            Ok(Value::List(out))
        }
        "flat" => {
            // Bir daraja tekislaydi: ichki list elementlari ochiladi, qolganlar
            // o'z holicha — chuqur rekursiya kerak bo'lsa flat'ni zanjirlash mumkin.
            let mut out = Vec::new();
            for x in xs {
                match x {
                    Value::List(inner) => out.extend(inner.iter().cloned()),
                    other => out.push(other.clone()),
                }
            }
            Ok(Value::List(out))
        }
        "zip" => {
            let other = arg(&args, 0, "list.zip")?;
            let Value::List(ys) = other else {
                return Err(Flow::err(format!(
                    "list.zip: argument list bo'lishi kerak, {} berildi",
                    other.type_name()
                )));
            };
            // Qisqasi tugaganda to'xtaydi — ortiqcha elementlar tashlanadi.
            Ok(Value::List(
                xs.iter()
                    .zip(ys)
                    .map(|(a, b)| Value::List(vec![a.clone(), b.clone()]))
                    .collect(),
            ))
        }
        // filter/map/reduce/find/any/all — funksiya argument oladi; interp uni
        // shu yerda chaqira olmaydi (apply Interp'da). Shuning uchun bu metodlar
        // maxsus ishlov talab qiladi — pastdagi izohga qarang.
        "filter" | "map" | "reduce" | "find" | "any" | "all" => Err(Flow::err(format!(
            "ichki: list.{} alohida yo'l bilan ishlov beriladi",
            method
        ))),
        _ => Err(Flow::err(format!("list metodi '{}' mavjud emas", method))),
    }
}

// Tabiiy tartibda saralash: son (int/flt aralash) va matn/sym bir jinsli
// bo'lsa ishlaydi; aralash tiplar uchun komparator berish talab qilinadi.
pub fn sort_default(xs: &[Value]) -> R {
    let sorted = sort_values(xs.to_vec(), &mut |a, b| {
        use std::cmp::Ordering;
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => Ok(x.cmp(y)),
            // NaN tartibsiz — Equal deb olamiz (saralash yiqilmasin).
            (Value::Flt(x), Value::Flt(y)) => Ok(x.partial_cmp(y).unwrap_or(Ordering::Equal)),
            (Value::Int(x), Value::Flt(y)) => {
                Ok((*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal))
            }
            (Value::Flt(x), Value::Int(y)) => {
                Ok(x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal))
            }
            (Value::Str(x), Value::Str(y)) => Ok(x.cmp(y)),
            (Value::Sym(x), Value::Sym(y)) => Ok(x.cmp(y)),
            (a, b) => Err(Flow::err(format!(
                "list.sort: {} va {} ni taqqoslab bo'lmaydi — komparator bering: l.sort \\a b -> ...",
                a.type_name(),
                b.type_name()
            ))),
        }
    })?;
    Ok(Value::List(sorted))
}

// Stable merge sort — std sort_by o'rniga, chunki komparator Flux funksiyasi
// bo'lganda xato (Flow) qaytishi mumkin: std sort xato yo'lida Equal qaytarsak
// "total order buzildi" deb panic qilishi mumkin. Bu yo'l xatoni toza ko'taradi.
pub fn sort_values<F>(mut xs: Vec<Value>, cmp: &mut F) -> Result<Vec<Value>, Flow>
where
    F: FnMut(&Value, &Value) -> Result<std::cmp::Ordering, Flow>,
{
    let len = xs.len();
    if len <= 1 {
        return Ok(xs);
    }
    let right = xs.split_off(len / 2);
    let left = sort_values(xs, cmp)?;
    let right = sort_values(right, cmp)?;
    let mut out = Vec::with_capacity(len);
    let mut li = left.into_iter().peekable();
    let mut ri = right.into_iter().peekable();
    loop {
        match (li.peek(), ri.peek()) {
            // Teng bo'lsa chap (asl tartibda oldingi) birinchi — stable.
            (Some(a), Some(b)) => {
                if cmp(a, b)? == std::cmp::Ordering::Greater {
                    out.push(ri.next().unwrap());
                } else {
                    out.push(li.next().unwrap());
                }
            }
            (Some(_), None) => out.push(li.next().unwrap()),
            (None, Some(_)) => out.push(ri.next().unwrap()),
            (None, None) => break,
        }
    }
    Ok(out)
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
// Timestamp argumentini unix sekundga o'qiydi: matn (ISO/kanonik, mintaqa ham)
// yoki to'g'ridan-to'g'ri unix int. time.fmt/add/diff bir xil kirishni qabul qilsin.
fn arg_ts(args: &[Value], i: usize, who: &str) -> Result<i64, Flow> {
    match arg(args, i, who)? {
        Value::Str(s) => parse_iso(s)
            .ok_or_else(|| Flow::err(format!("{}: timestamp matnini o'qib bo'lmadi: {}", who, s))),
        Value::Int(n) => Ok(*n),
        other => Err(Flow::err(format!(
            "{}: {}-argument timestamp (str/int) bo'lishi kerak, {} berildi",
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
mod rand_tests {
    use super::*;

    // rand.int chegaralar ichida qoladi (a..=b), span=1 ham (a==b).
    #[test]
    fn int_in_range() {
        for _ in 0..1000 {
            let Ok(Value::Int(v)) = rand_module("int", vec![Value::Int(10), Value::Int(20)]) else {
                panic!("rand.int int qaytarishi kerak");
            };
            assert!((10..=20).contains(&v), "diapazondan tashqari: {}", v);
        }
        let Ok(Value::Int(v)) = rand_module("int", vec![Value::Int(7), Value::Int(7)]) else {
            panic!("rand.int int qaytarishi kerak");
        };
        assert_eq!(v, 7); // span=1 -> doim a
    }

    // rand.str so'ralgan uzunlikda va faqat alfanumerik belgilardan iborat.
    #[test]
    fn str_len_and_alphabet() {
        let Ok(Value::Str(s)) = rand_module("str", vec![Value::Int(32)]) else {
            panic!("rand.str matn qaytarishi kerak");
        };
        assert_eq!(s.chars().count(), 32);
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    // Issue #89: chekka chegaralarda span hisobi (b - a + 1) i64 dan toshardi
    // va panic berardi. Endi i128 oraliq — to'liq i64 diapazoni ham ishlaydi.
    #[test]
    fn int_extreme_bounds_no_overflow() {
        for &(a, b) in &[
            (i64::MIN, i64::MAX),     // span = 2^64 (u64 ga ham sig'maydi)
            (i64::MIN, i64::MIN + 5), // juda manfiy tor diapazon
            (i64::MAX - 5, i64::MAX), // juda musbat tor diapazon
            (-3, i64::MAX),           // span > i64::MAX
        ] {
            for _ in 0..50 {
                let Ok(Value::Int(v)) = rand_module("int", vec![Value::Int(a), Value::Int(b)])
                else {
                    panic!("rand.int int qaytarishi kerak ({}..{})", a, b);
                };
                assert!((a..=b).contains(&v), "diapazondan tashqari: {}", v);
            }
        }
    }

    // Kriptografik manba: ketma-ket ikki token bir xil emas (bashorat qilinmas).
    // Eski xorshift'da bir nanosekundda ochilgan thread'lar bir xil olardi.
    #[test]
    fn tokens_are_unique() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for _ in 0..100 {
            let Ok(Value::Str(s)) = rand_module("str", vec![Value::Int(24)]) else {
                panic!("rand.str matn qaytarishi kerak");
            };
            assert!(seen.insert(s), "takror token chiqdi — CSPRNG buzildi");
        }
    }
}

#[cfg(test)]
mod math_tests {
    use super::*;

    // Issue #89: i64::MIN.abs() panic berardi (musbat juftligi i64 ga sig'maydi).
    // Endi Flux xatosi; oddiy qiymatlar avvalgidek ishlaydi.
    #[test]
    fn abs_min_is_error_not_panic() {
        let r = math_module("abs", vec![Value::Int(i64::MIN)]);
        let Err(Flow::Error(msg)) = r else {
            panic!("math.abs i64::MIN xato berishi kerak");
        };
        assert!(msg.contains("son chegaradan oshdi"), "xato matni: {}", msg);
        assert!(matches!(
            math_module("abs", vec![Value::Int(-7)]),
            Ok(Value::Int(7))
        ));
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

    #[test]
    fn in_adds_units() {
        // time.in kelajakni, time.ago o'tmishni beradi — natija hozirdan keyin/oldin.
        let now = now_unix();
        let Ok(Value::Str(f)) = time_module("in", vec![Value::Int(1), Value::Str("hr".into())])
        else {
            panic!("time.in matn qaytarishi kerak");
        };
        let Ok(Value::Str(p)) = time_module("ago", vec![Value::Int(1), Value::Str("hr".into())])
        else {
            panic!("time.ago matn qaytarishi kerak");
        };
        let (Some(fu), Some(pu)) = (parse_ts(&f), parse_ts(&p)) else {
            panic!("timestamp'larni o'qib bo'lmadi");
        };
        // 1 soat keyin > hozir > 1 soat oldin (sekundlik yumaloqlash chetga surilmaydi).
        assert!(
            fu >= now + 3600 - 1 && fu <= now + 3600 + 1,
            "time.in noto'g'ri: {}",
            f
        );
        assert!(
            pu >= now - 3600 - 1 && pu <= now - 3600 + 1,
            "time.ago noto'g'ri: {}",
            p
        );
    }

    #[test]
    fn in_rejects_bad_unit() {
        let r = time_module("in", vec![Value::Int(1), Value::Str("year".into())]);
        assert!(r.is_err(), "noma'lum birlik xato berishi kerak");
    }

    #[test]
    fn sleep_waits_and_returns_nil() {
        use std::time::Instant;
        // Qisqa flt kechikish — int emas, kasr ham qabul qilinishini tekshiramiz.
        let t0 = Instant::now();
        let r = time_module("sleep", vec![Value::Flt(0.05)]);
        let elapsed = t0.elapsed();
        assert!(
            matches!(r, Ok(Value::Nil)),
            "time.sleep nil qaytarishi kerak"
        );
        assert!(
            elapsed.as_millis() >= 45,
            "time.sleep kamida kutilgan vaqtni kutishi kerak: {:?}",
            elapsed
        );
    }

    #[test]
    fn sleep_negative_clamps_to_zero() {
        // Manfiy qiymat panic bermasligi kerak — 0 ga klamp qilinadi.
        let r = time_module("sleep", vec![Value::Int(-5)]);
        assert!(matches!(r, Ok(Value::Nil)), "manfiy sleep nil qaytarsin");
    }

    #[test]
    fn parse_iso_handles_z_and_offsets() {
        // "Z" -> UTC; "+HH:MM"/"-HH:MM" mintaqa UTC ga keltiriladi.
        let z = parse_iso("2026-06-10T10:00:00Z").expect("Z o'qilsin");
        assert_eq!(parse_iso("2026-06-10 10:00:00"), Some(z)); // mintaqasiz = UTC
        // +05:00: matndagi vaqt mahalliy, UTC 5 soat oldin.
        assert_eq!(parse_iso("2026-06-10T15:00:00+05:00"), Some(z));
        // -05:00: UTC 5 soat keyin.
        assert_eq!(parse_iso("2026-06-10T05:00:00-05:00"), Some(z));
        // "+HHMM" (ikki nuqtasiz) va kasr sekund ham o'qilsin.
        assert_eq!(parse_iso("2026-06-10T15:00:00.123+0500"), Some(z));
    }

    #[test]
    fn time_parse_normalizes_to_canonical_utc() {
        // time.parse ISO matnni kanonik "YYYY-MM-DD HH:MM:SS" UTC ga keltiradi.
        let Ok(Value::Str(s)) =
            time_module("parse", vec![Value::Str("2026-06-10T10:00:00Z".into())])
        else {
            panic!("time.parse matn qaytarishi kerak");
        };
        assert_eq!(s, "2026-06-10 10:00:00");
    }

    #[test]
    fn time_parse_rejects_garbage() {
        let r = time_module("parse", vec![Value::Str("salom".into())]);
        assert!(r.is_err(), "noto'g'ri matn xato berishi kerak");
    }

    #[test]
    fn parse_ts_rejects_impossible_dates() {
        // Mavjud bo'lmagan sana/vaqt jimgina "tuzatilmasin" — rad etilsin
        // (days_from_civil overflow'ni normalizatsiya qiladi, biz oldini olamiz).
        assert_eq!(parse_ts("2026-02-31T10:00:00Z"), None); // fevralda 31 yo'q
        assert_eq!(parse_ts("2026-02-29 00:00:00"), None); // 2026 kabisa emas
        assert_eq!(parse_ts("2026-13-01 00:00:00"), None); // 13-oy yo'q
        assert_eq!(parse_ts("2026-00-10 00:00:00"), None); // 0-oy yo'q
        assert_eq!(parse_ts("2026-06-00 00:00:00"), None); // 0-kun yo'q
        assert_eq!(parse_ts("2026-06-10 24:00:00"), None); // 24-soat yo'q
        assert_eq!(parse_ts("2026-06-10 10:60:00"), None); // 60-daqiqa yo'q
        // Haqiqiy chekka holatlar QABUL qilinadi:
        assert!(parse_ts("2024-02-29 00:00:00").is_some()); // 2024 kabisa
        assert!(parse_ts("2026-12-31 23:59:60").is_some()); // kabisa sekund (60)
    }

    #[test]
    fn time_parse_rejects_impossible_date() {
        let r = time_module("parse", vec![Value::Str("2026-02-31T10:00:00Z".into())]);
        assert!(r.is_err(), "02-31 mavjud emas — xato berishi kerak");
    }

    #[test]
    fn time_add_offsets_arbitrary_timestamp() {
        // Issue #65 yadrosi: start_at + duration -> end_at.
        let Ok(Value::Str(end)) = time_module(
            "add",
            vec![
                Value::Str("2026-06-10 10:00:00".into()),
                Value::Int(30),
                Value::Str("min".into()),
            ],
        ) else {
            panic!("time.add matn qaytarishi kerak");
        };
        assert_eq!(end, "2026-06-10 10:30:00");
        // Manfiy N orqaga siljitadi.
        let Ok(Value::Str(before)) = time_module(
            "add",
            vec![
                Value::Str("2026-06-10 10:00:00".into()),
                Value::Int(-2),
                Value::Str("hr".into()),
            ],
        ) else {
            panic!("time.add matn qaytarishi kerak");
        };
        assert_eq!(before, "2026-06-10 08:00:00");
    }

    #[test]
    fn time_add_rejects_bad_unit() {
        let r = time_module(
            "add",
            vec![
                Value::Str("2026-06-10 10:00:00".into()),
                Value::Int(1),
                Value::Str("year".into()),
            ],
        );
        assert!(r.is_err(), "noma'lum birlik xato berishi kerak");
    }

    // Issue #89: n * secs ko'paytmasi (yoki yakuniy yig'indi) i64 dan toshsa
    // panic/jim wrap emas, Flux xatosi qaytadi — to'rttala offset funksiyada.
    #[test]
    fn time_offsets_overflow_is_error() {
        let big = Value::Int(i64::MAX / 2);
        let day = Value::Str("day".into());
        for func in ["ago", "in"] {
            let r = time_module(func, vec![big.clone(), day.clone()]);
            let Err(Flow::Error(msg)) = r else {
                panic!("time.{} overflow'da xato berishi kerak", func);
            };
            assert!(msg.contains("son chegaradan oshdi"), "xato matni: {}", msg);
        }
        let base = Value::Str("2026-06-10 10:00:00".into());
        for func in ["add", "sub"] {
            let r = time_module(func, vec![base.clone(), big.clone(), day.clone()]);
            let Err(Flow::Error(msg)) = r else {
                panic!("time.{} overflow'da xato berishi kerak", func);
            };
            assert!(msg.contains("son chegaradan oshdi"), "xato matni: {}", msg);
        }
    }

    #[test]
    fn time_sub_offsets_backward() {
        // time.sub — add ning ko'zgusi: berilgan vaqtdan orqaga siljitadi.
        let Ok(Value::Str(s)) = time_module(
            "sub",
            vec![
                Value::Str("2026-06-10 10:00:00".into()),
                Value::Int(5),
                Value::Str("min".into()),
            ],
        ) else {
            panic!("time.sub matn qaytarishi kerak");
        };
        assert_eq!(s, "2026-06-10 09:55:00");
    }

    #[test]
    fn time_diff_returns_seconds() {
        // diff a b = a - b sekundda; musbat = a kelajakda.
        let r = time_module(
            "diff",
            vec![
                Value::Str("2026-06-10 10:30:00".into()),
                Value::Str("2026-06-10 10:00:00".into()),
            ],
        );
        assert!(matches!(r, Ok(Value::Int(1800))), "30 daqiqa = 1800 sek");
        // Teskari tartib manfiy beradi.
        let r = time_module(
            "diff",
            vec![
                Value::Str("2026-06-10 10:00:00".into()),
                Value::Str("2026-06-10 10:30:00".into()),
            ],
        );
        assert!(matches!(r, Ok(Value::Int(-1800))));
    }

    #[test]
    fn time_diff_accepts_iso_with_offset() {
        // Aralash format: biri ISO mintaqali, biri kanonik — ikkisi ham UTC ga keladi.
        let r = time_module(
            "diff",
            vec![
                Value::Str("2026-06-10T15:30:00+05:00".into()), // = 10:30 UTC
                Value::Str("2026-06-10 10:00:00".into()),
            ],
        );
        assert!(matches!(r, Ok(Value::Int(1800))));
    }

    #[test]
    fn parse_in_zone_is_dst_aware() {
        // Bir xil wall-clock (12:00 local) DST'da turli UTC offset beradi:
        // qishda America/New_York = UTC-5 (EST), yozda UTC-4 (EDT). Fiksrlangan
        // offset DEB hisoblamaslik isboti — issue #80 yadrosi.
        let winter = parse_in_zone("2026-01-15 12:00:00", "America/New_York").unwrap();
        assert_eq!(fmt_unix(winter), "2026-01-15 17:00:00"); // EST: +5 UTC
        let summer = parse_in_zone("2026-07-15 12:00:00", "America/New_York").unwrap();
        assert_eq!(fmt_unix(summer), "2026-07-15 16:00:00"); // EDT: +4 UTC
    }

    #[test]
    fn parse_in_zone_rejects_spring_forward_gap() {
        // 2026-03-08 02:00 -> 03:00 sakraydi: 02:30 local mavjud emas -> None.
        assert_eq!(
            parse_in_zone("2026-03-08 02:30:00", "America/New_York"),
            None
        );
    }

    #[test]
    fn parse_in_zone_rejects_unknown_zone() {
        assert_eq!(parse_in_zone("2026-01-15 12:00:00", "Mars/Olympus"), None);
    }

    #[test]
    fn fmt_in_zone_converts_utc_to_local() {
        // UTC instant -> zona wall-clock (DST hisobga olinib).
        let winter = parse_in_zone("2026-01-15 12:00:00", "America/New_York").unwrap();
        assert_eq!(
            fmt_in_zone(winter, "YYYY-MM-DD HH:mm", "America/New_York").unwrap(),
            "2026-01-15 12:00"
        );
        // Asia/Tashkent doimiy +5 (DST yo'q) — 17:00 UTC -> 22:00 local.
        let utc = parse_ts("2026-06-10 17:00:00").unwrap();
        assert_eq!(fmt_in_zone(utc, "HH:mm", "Asia/Tashkent").unwrap(), "22:00");
    }

    #[test]
    fn time_parse_with_zone_module_level() {
        // time.parse'ning ixtiyoriy 2-argument (zona) yo'li UTC kanonik beradi.
        let Ok(Value::Str(s)) = time_module(
            "parse",
            vec![
                Value::Str("2026-07-15 09:00:00".into()),
                Value::Str("America/New_York".into()),
            ],
        ) else {
            panic!("time.parse zona bilan matn qaytarishi kerak");
        };
        assert_eq!(s, "2026-07-15 13:00:00"); // EDT (+4) -> UTC
    }

    #[test]
    fn time_fmt_with_zone_module_level() {
        // time.fmt'ning ixtiyoriy 3-argument (zona) local wall-clock beradi.
        let Ok(Value::Str(s)) = time_module(
            "fmt",
            vec![
                Value::Str("2026-07-15 13:00:00".into()),
                Value::Str("HH:mm".into()),
                Value::Str("America/New_York".into()),
            ],
        ) else {
            panic!("time.fmt zona bilan matn qaytarishi kerak");
        };
        assert_eq!(s, "09:00"); // 13:00 UTC -> EDT 09:00
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

#[cfg(test)]
mod json_tests {
    use super::*;

    // Control belgilar (0x00–0x1F) \u00XX shaklida escape bo'lishi kerak —
    // issue #102: avval 0x08 kabilar xom chiqib invalid JSON berardi.
    #[test]
    fn control_chars_escaped() {
        let s = Value::Str("a\u{08}b\u{01}c".into());
        // 0x08 -> \b (qisqa shakl), 0x01 -> umumiy \u escape 
        assert_eq!(json_encode(&s), "\"a\\bb\\u0001c\"");
    }

    // \f va \b qisqa shaklda; round-trip dekoder bilan ishlashi kerak.
    #[test]
    fn backspace_formfeed_roundtrip() {
        let s = Value::Str("x\u{0C}y\u{08}z".into());
        let enc = json_encode(&s);
        assert_eq!(enc, "\"x\\fy\\bz\"");
        match json_decode(&enc) {
            Ok(Value::Str(out)) => assert_eq!(out, "x\u{0C}y\u{08}z"),
            other => panic!("round-trip buzildi: {:?}", other.is_ok()),
        }
    }

    // Infinity/NaN -> null (JSON.stringify xulqi), "inf"/"NaN" emas.
    #[test]
    fn non_finite_floats_become_null() {
        assert_eq!(json_encode(&Value::Flt(f64::INFINITY)), "null");
        assert_eq!(json_encode(&Value::Flt(f64::NEG_INFINITY)), "null");
        assert_eq!(json_encode(&Value::Flt(f64::NAN)), "null");
        // oddiy float o'zgarishsiz qoladi
        assert_eq!(json_encode(&Value::Flt(1.5)), "1.5");
    }

    // Dekoder: qiymatdan keyin chiqindi xato beradi (avval jim qabul qilinardi).
    #[test]
    fn trailing_garbage_rejected() {
        assert!(json_decode("1 garbage").is_err());
        assert!(json_decode("{} extra").is_err());
        // bo'sh joy bilan tugagan to'g'ri JSON esa qabul qilinadi
        assert!(matches!(json_decode("1  \n"), Ok(Value::Int(1))));
    }

    // Dekoder: noto'g'ri `null`-o'xshash matn xato beradi (avval nil berardi).
    #[test]
    fn invalid_null_rejected() {
        assert!(json_decode("nqqq").is_err());
        assert!(matches!(json_decode("null"), Ok(Value::Nil)));
    }

    // Dekoder: yaroqsiz sonlar rad etiladi (boshida '+', ikkita nuqta...).
    #[test]
    fn strict_number_grammar() {
        assert!(json_decode("+5").is_err());
        assert!(json_decode("1.2.3").is_err());
        assert!(json_decode("01").is_err());
        assert!(json_decode("1.").is_err());
        assert!(json_decode("1e").is_err());
        // to'g'ri sonlar ishlaydi
        assert!(matches!(json_decode("-5"), Ok(Value::Int(-5))));
        assert!(matches!(json_decode("1.5e3"), Ok(Value::Flt(_))));
        assert!(matches!(json_decode("0"), Ok(Value::Int(0))));
    }

    // Dekoder: kesilgan/buzuq JSON panic emas, xato qaytaradi (issue #87).
    // Tashqi input (HTTP body) interpreterni yiqitmasligi shart.
    #[test]
    fn truncated_json_no_panic() {
        // satr ochilib tugamasdan kesilgan: `{` -> kalit kutilgan joyda tugash
        assert!(json_decode("{").is_err());
        // ochilib yopilmagan satr
        assert!(json_decode("\"").is_err());
        assert!(json_decode("\"ab").is_err());
        // satr `\` bilan tugab qolgan (escape baytini o'qishda chegaradan o'tish)
        assert!(json_decode("\"ab\\").is_err());
        // ochilib tugamagan massiv/obyekt ham xato
        assert!(json_decode("[").is_err());
        assert!(json_decode("[1,").is_err());
        assert!(json_decode("{\"k\"").is_err());
    }
}
