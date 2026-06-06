// Flux cron battery — rejalashtirilgan fon vazifalari.
//
// Til API (docs):
//   cron.on 0 * * * * check_prices     # daqiqa soat kun oy hafta-kuni; nomli funksiya
//   cron.on 30 9 * * * \-> ...          # inline lambda (parametrsiz)
//   cron.run                            # server YO'Q bo'lsa: processni ushlab tur
//
// Sintaksis: standart Unix 5-maydonli cron ifoda. Parser uni TIRNOQSIZ o'qiydi
// (`*` ko'paytirish emas) — `cron.on` callee'si ko'rilganda parser.rs maxsus 5
// maydonni str'ga yig'adi. Tirnoqli variant (`cron.on "0 * * * *" f`) ham ishlaydi.
//
// Model (foydalanuvchi qarori): `cron.on` HECH QACHON bloklamaydi — `http.on`/
// `ws.on` kabi faqat ro'yxatga oladi va scheduler fon thread'ini (bir marta)
// yoqadi. Boshqa bloklovchi process (`http.serve`/`ws.serve`) bo'lsa, cron fonda
// ishlayveradi. Faqat-cron skript uchun `cron.run` processni o'z qo'liga oladi.
//
// Scheduler — oddiy `std::thread` + uxlash sikli (tokio EMAS): cron handler'lari
// sinxron tree-walking, async kerak emas. Har handler `apply` orqali argumentsiz
// chaqiriladi; xato serverni o'ldirmaydi (ws fire_handler kabi stderr diagnostika).

use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::{Timelike, Utc};
use cron::Schedule;
use parking_lot::Mutex;

use crate::interp::{Flow, Interp};
use crate::value::Value;

// Bitta ro'yxatga olingan vazifa: parse qilingan jadval + chaqiriladigan handler.
struct CronJob {
    schedule: Schedule,
    handler: Value,
}

// cron battery holati — jarayonga bitta (Interp ichida Arc). Top-level kod
// (`cron.on`) `jobs` ni to'ldiradi, scheduler fon thread'i o'qiydi.
pub struct CronState {
    // Ro'yxatga olingan vazifalar. `cron.on` push qiladi, scheduler iteratsiya qiladi.
    jobs: Mutex<Vec<CronJob>>,
    // Scheduler thread BIR marta yonishi uchun marker (idempotent start).
    started: OnceLock<()>,
}

impl CronState {
    pub fn new() -> Self {
        CronState {
            jobs: Mutex::new(Vec::new()),
            started: OnceLock::new(),
        }
    }
}

impl Default for CronState {
    fn default() -> Self {
        Self::new()
    }
}

// `cron` ifodasi (str) -> Schedule.
//
// Flux standart Unix 5-maydonli format (daqiqa soat kun oy hafta-kuni) ishlatadi,
// `cron` crate esa 6-7 maydonli (sekund bilan) kutadi — oldiga "0 " (sekund=0)
// qo'shamiz. Yana bir nomuvofiqlik: Unix cron'da hafta-kuni `0`=yakshanba (va `7`
// ham yakshanba), `cron` crate esa faqat `1-7`/`SUN-SAT` qabul qiladi (`0` xato).
// Shuning uchun hafta-kuni maydonidagi har bir yakka `0` ni `7` ga aylantiramiz —
// shunda `0 18 * * 0` (yakshanba 18:00) standart Unix kabi ishlaydi.
fn parse_schedule(expr: &str) -> Result<Schedule, Flow> {
    let trimmed = expr.trim();
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    // 5 maydon (standart Unix) bo'lsa sekund maydonini old qo'shamiz va hafta-kunini
    // normalizatsiya qilamiz. Boshqa maydon soni bo'lsa o'zgartirmaymiz (crate o'zi
    // tekshiradi).
    let normalized = if fields.len() == 5 {
        let weekday = normalize_weekday(fields[4]);
        format!(
            "0 {} {} {} {} {weekday}",
            fields[0], fields[1], fields[2], fields[3]
        )
    } else {
        trimmed.to_string()
    };
    Schedule::from_str(&normalized)
        .map_err(|e| Flow::err(format!("cron.on: noto'g'ri cron ifoda '{expr}': {e}")))
}

// Unix hafta-kuni `0`(yakshanba) -> cron crate `7`(yakshanba). Maydonni `,` (ro'yxat)
// bo'yicha bo'lib, aynan `0` bo'lgan a'zolarni `7` qiladi (`* * * * 0` va `* * * * 0,6`
// kabi keng tarqalgan holatlar). Diapazon BOSHIDAGI `0` (`0-2`) — crate'da `7-2`
// teskari bo'lib qolardi, shuning uchun diapazonni `7` + qolgan qism ko'rinishida
// kengaytiramiz (`0-2` -> `7,1-2`; `0-0` -> `7`). `10`,`20` kabi raqamlarga tegmaydi.
fn normalize_weekday(field: &str) -> String {
    field
        .split(',')
        .map(normalize_weekday_member)
        .collect::<Vec<_>>()
        .join(",")
}

// Bitta ro'yxat a'zosi (yakka qiymat yoki diapazon) ni normalizatsiya qiladi.
fn normalize_weekday_member(part: &str) -> String {
    // Yakka `0` -> `7`.
    if part == "0" {
        return "7".to_string();
    }
    // Diapazon `A-B`: faqat A==0 holatini maxsus kengaytiramiz.
    if let Some((a, b)) = part.split_once('-')
        && a == "0"
    {
        // `0-0` -> yakshanba; `0-N` -> yakshanba (7) + dushanba..N (1-N).
        return if b == "0" {
            "7".to_string()
        } else {
            format!("7,1-{b}")
        };
    }
    part.to_string()
}

impl Interp {
    // cron.<func> chaqiruvlari.
    pub fn cron_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "on" => self.cron_on(args),
            "run" => self.cron_run(args),
            _ => Err(Flow::err(format!("cron modulida '{func}' funksiyasi yo'q"))),
        }
    }

    // cron.on <ifoda> <handler> — vazifani ro'yxatga oladi va schedulerni yoqadi.
    // Bloklamaydi (http.on kabi). Birinchi `cron.on` da fon thread'i yonadi.
    fn cron_on(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let expr = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => {
                return Err(Flow::err(
                    "cron.on: 1-argument cron ifoda (masalan 0 * * * *) bo'lishi kerak",
                ));
            }
        };
        let handler = match args.get(1) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => return Err(Flow::err("cron.on: 2-argument handler (fn) bo'lishi kerak")),
        };
        let schedule = parse_schedule(&expr)?;
        self.cron.jobs.lock().push(CronJob { schedule, handler });
        // Scheduler fon thread'ini yoqamiz (bir marta). cron.on bloklamaydi.
        self.start_scheduler();
        Ok(Value::Nil)
    }

    // cron.run — processni ushlab turish kerakligini bildiradi (DARHOL bloklamaydi).
    // Scheduler allaqachon fon thread'da ishlaydi; cron.run faqat "top-level tugagach
    // dastur tugamasin" belgisini qo'yadi. Bu http.serve/ws.serve kabi DEFERRED:
    // `pending_servers`ga Cron qo'shiladi, top-level tugagach `run_pending` ushlab
    // turadi. Shunda `cron.run` + `http.serve` ixtiyoriy tartibda birga ishlaydi —
    // ilgari cron.run `loop { sleep }` bilan o'zidan keyingi serve'ni bloklardi.
    fn cron_run(self: &Arc<Self>, _args: Vec<Value>) -> Result<Value, Flow> {
        self.start_scheduler(); // hech qanday cron.on bo'lmagan bo'lsa ham (no-op jobs)
        self.pending_servers
            .lock()
            .unwrap()
            .push(crate::serve_mod::PendingServer::Cron);
        Ok(Value::Nil)
    }

    // Scheduler fon thread'ini bir marta yoqadi.
    //
    // MUHIM: bu yerda `freeze_globals` CHAQIRILMAYDI. `cron.on` top-level kod
    // O'RTASIDA chaqiriladi (undan keyin yana global binding bo'lishi mumkin) —
    // o'sha paytda muzlatsak, keyingi global o'zgaruvchilar snapshot'ga tushmay
    // qoladi va ularga murojaat "noma'lum nom" beradi. Scheduler thread global'ni
    // RwLock orqali o'qiydi (lookup muzlatilmagan holatni qo'llaydi); cron daqiqada
    // bir marta ishlagani uchun bu sekinlik ahamiyatsiz. Agar keyin `http.serve`/
    // `ws.serve` chaqirilsa, ULAR muzlatadi va cron ham frozen snapshot'dan o'qiydi.
    fn start_scheduler(self: &Arc<Self>) {
        // OnceLock::set faqat birinchi marta muvaffaqiyatli — ikkinchi cron.on jim o'tadi.
        if self.cron.started.set(()).is_err() {
            return;
        }
        let interp = self.clone();
        std::thread::spawn(move || run_scheduler(interp));
    }
}

// Scheduler sikli: har daqiqa chegarasida uyg'onib, joriy daqiqaga to'g'ri keladigan
// vazifalarni ishga tushiradi. cron 5-maydon daqiqa aniqligida bo'lgani uchun
// daqiqalik granularity yetarli.
fn run_scheduler(interp: Arc<Interp>) {
    // Oxirgi ishga tushirilgan daqiqa (epoch daqiqa) — bir daqiqada ikki marta
    // ishga tushirmaslik uchun har job uchun alohida kuzatamiz.
    loop {
        // Keyingi daqiqa boshigacha uxlaymiz (joriy soniya 0 bo'lganda tekshiramiz).
        let now = Utc::now();
        let secs_into_minute = now.second();
        let sleep_secs = 60u64.saturating_sub(secs_into_minute as u64).max(1);
        std::thread::sleep(Duration::from_secs(sleep_secs));

        // Joriy daqiqa boshini (sekund=0) referens nuqta qilamiz.
        let tick = Utc::now().with_second(0).and_then(|t| t.with_nanosecond(0));
        let Some(tick) = tick else { continue };

        // Har job: oldingi daqiqadan keyingi run aynan shu daqiqaga tushsa — ishga tushir.
        let jobs = interp.cron.jobs.lock();
        for job in jobs.iter() {
            // `after(tick - 1min)` keyingi run >= tick bo'lsa va == tick bo'lsa mos keladi.
            let prev = tick - chrono::Duration::minutes(1);
            if let Some(next) = job.schedule.after(&prev).next()
                && next == tick
            {
                let interp = interp.clone();
                let handler = job.handler.clone();
                // Handler'ni alohida thread'da chaqiramiz — uzoq ishlasa scheduler
                // siklini bloklamaydi (keyingi daqiqa o'z vaqtida tekshiriladi).
                std::thread::spawn(move || {
                    if let Err(flow) = interp.apply(handler, vec![]) {
                        eprintln!("cron handler xatosi: {}", flow_msg(&flow));
                    }
                });
            }
        }
    }
}

fn flow_msg(flow: &Flow) -> String {
    match flow {
        Flow::Fail { message, .. } => message.clone(),
        Flow::Error(e) => e.clone(),
        _ => "skip/stop/return".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standart_5_maydon_parse() {
        // Har soat boshida.
        assert!(parse_schedule("0 * * * *").is_ok());
        // Har 15 daqiqada.
        assert!(parse_schedule("*/15 * * * *").is_ok());
        // Ish kunlari 09:30.
        assert!(parse_schedule("30 9 * * 1-5").is_ok());
        // Ro'yxat.
        assert!(parse_schedule("0 0,12 * * *").is_ok());
        // Unix hafta-kuni 0 = yakshanba (cron crate 0 ni rad etadi, biz 7 ga aylantiramiz).
        assert!(parse_schedule("0 18 * * 0").is_ok());
        // Yakshanba diapazon/ro'yxat a'zosi sifatida.
        assert!(parse_schedule("0 0 * * 0,3").is_ok());
        assert!(parse_schedule("0 0 * * 0-2").is_ok());
    }

    #[test]
    fn hafta_kuni_0_yakshanba() {
        // `* * * * 0` (Unix yakshanba) va `* * * * 7` (crate yakshanba) bir xil
        // kunga tushishi kerak — normalize_weekday 0 -> 7 to'g'ri ishlaganini tasdiqlaydi.
        // Flow Debug derive qilmaydi -> expect() o'rniga match.
        let (Ok(s0), Ok(s7)) = (parse_schedule("0 12 * * 0"), parse_schedule("0 12 * * 7")) else {
            panic!("yakshanba 0/7 parse bo'lishi kerak edi");
        };
        let now = Utc::now();
        let next0 = s0.after(&now).next();
        let next7 = s7.after(&now).next();
        assert_eq!(next0, next7, "0 va 7 bir xil yakshanbaga tushishi kerak");
    }

    #[test]
    fn notogri_ifoda_xato() {
        // 99-daqiqa yo'q.
        assert!(parse_schedule("99 * * * *").is_err());
        // Bo'sh.
        assert!(parse_schedule("").is_err());
        // Maydon yetishmaydi.
        assert!(parse_schedule("* *").is_err());
    }

    #[test]
    fn keyingi_run_hisoblanadi() {
        // Har daqiqa ishlaydigan jadval — keyingi run hozirdan keyin bo'lishi kerak.
        // (Flow Debug derive qilmaydi, shuning uchun unwrap() o'rniga ok_or bilan.)
        let sched = match parse_schedule("* * * * *") {
            Ok(s) => s,
            Err(_) => panic!("'* * * * *' parse bo'lishi kerak edi"),
        };
        let now = Utc::now();
        let next = sched.after(&now).next();
        assert!(next.is_some());
        assert!(next.unwrap() > now);
    }
}
