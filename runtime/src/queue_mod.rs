// Flux queue battery — fon navbati (background jobs).
//
// Til API (docs):
//   queue.push "send" {ph:p body:t}            # navbatga ish qo'shadi (nom + payload map)
//   queue.on "send" \job -> tools.send job.ph job.body   # shu nomli ish uchun ishlovchi
//
// Falsafa (spec): webhook tez javob qaytarishi uchun og'ir ishni fonga uzatasiz.
// `queue.push` darhol qaytadi (bloklamaydi), ish fon worker thread'ida bajariladi.
//
// Model (foydalanuvchi qarori):
//   - BITTA worker thread, FIFO tartib. Ishlar ketma-ket bajariladi — tartib
//     kafolatlangan, beqaror yuk ostida ham thread portlamaydi. (`cron` kabi fon
//     thread, lekin u vaqt bo'yicha emas, navbat bo'yicha uyg'onadi.)
//   - Handler hali ro'yxatga olinmagan ish uchun `queue.push` chaqirilsa, ish
//     navbatda KUTIB turadi. Flux top-level kodda `queue.push` `queue.on` dan
//     oldin yozilishi mumkin — worker Condvar'da uxlaydi, `queue.on` kelganda
//     uyg'onib ishni bajaradi (busy-loop YO'Q, issue #105). Handler'siz ish
//     handler'i BOR boshqa ishlarni to'sib qo'ymaydi.
//   - Top-level kod tugaganda `queue_wait_drain` navbat bo'shashini kutadi —
//     ishlar jim yo'qolmaydi (issue #105). Handler'i hech qachon ro'yxatga
//     olinmagan ishlar shu yerda ogohlantirish bilan tashlanadi (ularni
//     bajarishning iloji yo'q — kutish abadiy bo'lardi).
//
// Worker — oddiy `std::thread` + `Condvar` (tokio EMAS): handler'lar sinxron
// tree-walking, async kerak emas. Handler `apply` orqali bitta `job` map argumenti
// bilan chaqiriladi; xato worker'ni o'ldirmaydi (cron/ws fire kabi stderr diagnostika).

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::OnceLock;

use parking_lot::{Condvar, Mutex};

use crate::interp::{Flow, Interp};
use crate::value::Value;

// Navbatdagi bitta ish: qaysi handler'ga (nom) va qanday ma'lumot bilan (payload).
struct Job {
    name: String,
    payload: Value,
}

// queue battery holati — jarayonga bitta (Interp ichida Arc). Top-level kod
// (`queue.push`/`queue.on`) to'ldiradi, worker fon thread'i o'qiydi.
pub struct QueueState {
    // FIFO navbat + nom->handler registri. Bir Mutex ostida — worker ikkalasini
    // birgalikda ko'radi (ishni olib, nomiga handler bor-yo'qligini bir lock'da
    // tekshiradi). Condvar yangi ish/handler kelganini worker'ga bildiradi.
    inner: Mutex<QueueInner>,
    not_empty: Condvar,
    // Worker bitta ishni tugatganini bildiradi — `queue_wait_drain` shu orqali
    // navbat bo'shashini kutadi (issue #105: ishlar jim yo'qolmasin).
    idle: Condvar,
    // Worker thread BIR marta yonishi uchun marker (idempotent start).
    started: OnceLock<()>,
}

struct QueueInner {
    queue: VecDeque<Job>,
    handlers: HashMap<String, Value>,
    // Worker hozir bitta ishni bajaryaptimi — drain navbat bo'sh bo'lsa ham
    // bajarilayotgan ish tugashini kutishi kerak.
    busy: bool,
    // Handler'siz nom haqida stderr ogohlantirishi BIR marta chiqsin.
    warned: HashSet<String>,
}

impl QueueState {
    pub fn new() -> Self {
        QueueState {
            inner: Mutex::new(QueueInner {
                queue: VecDeque::new(),
                handlers: HashMap::new(),
                busy: false,
                warned: HashSet::new(),
            }),
            not_empty: Condvar::new(),
            idle: Condvar::new(),
            started: OnceLock::new(),
        }
    }
}

impl Default for QueueState {
    fn default() -> Self {
        Self::new()
    }
}

impl Interp {
    // queue.<func> chaqiruvlari.
    pub fn queue_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            "push" => self.queue_push(args),
            "on" => self.queue_on(args),
            _ => Err(Flow::err(format!(
                "queue modulida '{func}' funksiyasi yo'q"
            ))),
        }
    }

    // queue.push <nom> <payload> — navbatga ish qo'shadi va worker'ni yoqadi.
    // Bloklamaydi: ish darhol navbatga tushadi, worker fonda bajaradi. Payload
    // ixtiyoriy (berilmasa Nil) — handler bittagina `job` argumenti oladi.
    fn queue_push(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let name = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => {
                return Err(Flow::err(
                    "queue.push: 1-argument ish nomi (str) bo'lishi kerak",
                ));
            }
        };
        // Payload ixtiyoriy. Berilmasa bo'sh map o'rniga Nil — handler o'zi hal qiladi.
        let payload = args.get(1).cloned().unwrap_or(Value::Nil);
        {
            let mut inner = self.queue.inner.lock();
            inner.queue.push_back(Job { name, payload });
        }
        // Worker fon thread'ini yoqamiz (bir marta) va uxlayotgan bo'lsa uyg'otamiz.
        self.start_worker();
        self.queue.not_empty.notify_one();
        Ok(Value::Nil)
    }

    // queue.on <nom> <handler> — shu nomli ishlar uchun ishlovchini ro'yxatga oladi.
    // Bloklamaydi. Handler kutib turgan ishlar bo'lishi mumkin (push on'dan oldin
    // yozilgan) — worker'ni uyg'otamiz, u endi handler topib ularni bajaradi.
    fn queue_on(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let name = match args.first() {
            Some(Value::Str(s)) => s.clone(),
            _ => {
                return Err(Flow::err(
                    "queue.on: 1-argument ish nomi (str) bo'lishi kerak",
                ));
            }
        };
        let handler = match args.get(1) {
            Some(v @ (Value::Fn(_) | Value::Native(_))) => v.clone(),
            _ => {
                return Err(Flow::err(
                    "queue.on: 2-argument handler (fn) bo'lishi kerak",
                ));
            }
        };
        {
            let mut inner = self.queue.inner.lock();
            inner.handlers.insert(name, handler);
        }
        // Worker yonib turgan bo'lsin (faqat-queue skript uchun ham) va kutayotgan
        // ishlarni endi handler bilan qayta ko'rsin.
        self.start_worker();
        self.queue.not_empty.notify_one();
        Ok(Value::Nil)
    }

    // Worker fon thread'ini bir marta yoqadi.
    //
    // MUHIM (cron bilan bir xil): bu yerda `freeze_globals` CHAQIRILMAYDI.
    // `queue.push`/`queue.on` top-level kod O'RTASIDA chaqiriladi — o'sha paytda
    // muzlatsak, keyingi global o'zgaruvchilar snapshot'ga tushmay qoladi va
    // ularga murojaat "noma'lum nom" beradi. Worker global'ni RwLock orqali o'qiydi.
    // Keyin `http.serve`/`ws.serve` chaqirilsa, ULAR muzlatadi va worker ham frozen
    // snapshot'dan o'qiydi.
    fn start_worker(self: &Arc<Self>) {
        // OnceLock::set faqat birinchi marta muvaffaqiyatli — keyingilari jim o'tadi.
        if self.queue.started.set(()).is_err() {
            return;
        }
        let interp = self.clone();
        std::thread::spawn(move || run_worker(interp));
    }

    // Top-level kod tugaganda chaqiriladi (interp.rs `run`): navbatdagi bajarib
    // bo'ladigan ishlar va bajarilayotgan ish tugaguncha bloklaydi — skript
    // chiqishida fon ishlar jim yo'qolmasin (issue #105). Handler'i hech qachon
    // ro'yxatga olinmagan ishlarni bajarishning iloji yo'q — ogohlantirib
    // tashlaymiz (aks holda kutish abadiy bo'lardi). Handler ichidan push
    // qilingan yangi ishlar ham shu yerda kutiladi (predikat har uyg'onishda
    // qayta tekshiriladi).
    pub fn queue_wait_drain(&self) {
        let mut inner = self.queue.inner.lock();
        loop {
            let runnable = inner
                .queue
                .iter()
                .any(|j| inner.handlers.contains_key(&j.name));
            if !runnable && !inner.busy {
                break;
            }
            self.queue.idle.wait(&mut inner);
        }
        for job in inner.queue.drain(..) {
            eprintln!(
                "queue: '{}' ishi bajarilmadi — handler ro'yxatga olinmagan (process tugadi)",
                job.name
            );
        }
    }
}

// Worker sikli: navbatdan handler'i ro'yxatga olingan BIRINCHI ishni oladi va
// chaqiradi. Handler'i hali yo'q ish navbatda qoladi (eski "qayta qo'yish +
// 50ms uxlash" busy-loop'i emas — issue #105): worker Condvar'da uxlaydi,
// `queue.on`/`queue.push` uyg'otadi. Handler'siz ish handler'i bor ishlarni
// to'sib qo'ymaydi; bir nom ichida FIFO saqlanadi (bir nomli ishlarning handler
// holati bir xil — tanlash tartibni buzmaydi).
fn run_worker(interp: Arc<Interp>) {
    loop {
        let (job, handler) = {
            let mut inner = interp.queue.inner.lock();
            loop {
                // MutexGuard orqali maydonlarni alohida borrow qilish uchun.
                let st = &mut *inner;
                if let Some(pos) = st
                    .queue
                    .iter()
                    .position(|j| st.handlers.contains_key(&j.name))
                {
                    // pos hozirgina topildi — remove albatta Some qaytaradi.
                    let Some(job) = st.queue.remove(pos) else {
                        continue;
                    };
                    let handler = st.handlers[&job.name].clone();
                    // Drain navbat bo'sh bo'lsa ham shu ish tugashini kutsin.
                    st.busy = true;
                    break (job, handler);
                }
                // Bajarib bo'ladigan ish yo'q. Handler'siz kutayotgan nomlar
                // haqida BIR marta diagnostika beramiz (issue #105).
                let unknown: Vec<String> = st
                    .queue
                    .iter()
                    .map(|j| j.name.clone())
                    .filter(|n| !st.warned.contains(n))
                    .collect();
                for name in unknown {
                    eprintln!(
                        "queue: '{}' uchun handler yo'q (queue.on chaqirilmagan) — ish navbatda kutmoqda",
                        name
                    );
                    st.warned.insert(name);
                }
                interp.queue.not_empty.wait(&mut inner);
            }
        };

        // Handler'ni SHU thread'da, bitta `job` (payload) argumenti bilan
        // chaqiramiz — FIFO va ketma-ketlik shu bilan kafolatlanadi. Xato
        // worker'ni o'ldirmaydi.
        if let Err(flow) = interp.apply(handler, vec![job.payload]) {
            eprintln!("queue handler '{}' xatosi: {}", job.name, flow_msg(&flow));
        }
        // Ish tugadi — drain kutayotgan bo'lsa uyg'otamiz.
        interp.queue.inner.lock().busy = false;
        interp.queue.idle.notify_all();
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
    use std::time::Duration;

    use super::*;

    // queue.push payload'siz ham ishlashi kerak (payload Nil bo'ladi).
    #[test]
    fn push_payloadsiz_nil() {
        let interp = Arc::new(Interp::new());
        let r = interp.queue_push(vec![Value::Str("ish".into())]);
        assert!(r.is_ok());
        let inner = interp.queue.inner.lock();
        assert_eq!(inner.queue.len(), 1);
        match &inner.queue.front() {
            Some(job) => assert!(matches!(job.payload, Value::Nil)),
            None => panic!("ish navbatga tushishi kerak edi"),
        }
    }

    // queue.push 1-argument str bo'lmasa xato.
    #[test]
    fn push_nom_str_bolmasa_xato() {
        let interp = Arc::new(Interp::new());
        assert!(interp.queue_push(vec![Value::Int(5)]).is_err());
        assert!(interp.queue_push(vec![]).is_err());
    }

    // queue.on handler fn bo'lmasa xato; nom str bo'lmasa xato.
    #[test]
    fn on_argument_tekshiruvi() {
        let interp = Arc::new(Interp::new());
        // Handler yo'q.
        assert!(interp.queue_on(vec![Value::Str("ish".into())]).is_err());
        // Handler fn emas.
        assert!(
            interp
                .queue_on(vec![Value::Str("ish".into()), Value::Int(1)])
                .is_err()
        );
        // Nom str emas.
        assert!(interp.queue_on(vec![Value::Int(1)]).is_err());
    }

    // queue.on handler'ni registr'ga qo'shadi.
    #[test]
    fn on_handler_royxatga_oladi() {
        let interp = Arc::new(Interp::new());
        let handler = Value::Native(Arc::new(crate::value::NativeFn {
            name: "noop".into(),
            func: Box::new(|_| Ok(Value::Nil)),
        }));
        let r = interp.queue_on(vec![Value::Str("ish".into()), handler]);
        assert!(r.is_ok());
        let inner = interp.queue.inner.lock();
        assert!(inner.handlers.contains_key("ish"));
    }

    // queue modulida noma'lum funksiya xato beradi.
    #[test]
    fn nomalum_funksiya_xato() {
        let interp = Arc::new(Interp::new());
        assert!(interp.queue_dispatch("yoq", vec![]).is_err());
    }

    // Uchma-uch: queue.on handler ro'yxatga olinadi, queue.push ish qo'shadi va
    // worker fon thread'i handler'ni HAQIQATAN chaqiradi (payload bilan). Native
    // handler `AtomicUsize`'ni oshiradi — asosiy thread shu counter orqali ishni
    // kuzatadi. Bu Condvar/worker oqimini to'g'ridan-to'g'ri tekshiradi.
    #[test]
    fn worker_handlerni_haqiqatan_chaqiradi() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let interp = Arc::new(Interp::new());
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let handler = Value::Native(Arc::new(crate::value::NativeFn {
            name: "inc".into(),
            func: Box::new(move |args| {
                // Payload bizga yetib kelganini ham tasdiqlaymiz (Int(7) kutamiz).
                if matches!(args.first(), Some(Value::Int(7))) {
                    c.fetch_add(1, Ordering::SeqCst);
                }
                Ok(Value::Nil)
            }),
        }));
        // Handler oldin ro'yxatga olinadi, keyin 3 ta ish push qilinadi.
        // (Flow Debug derive qilmaydi -> expect() o'rniga is_ok() bilan tasdiqlaymiz.)
        assert!(
            interp
                .queue_on(vec![Value::Str("ish".into()), handler])
                .is_ok()
        );
        for _ in 0..3 {
            assert!(
                interp
                    .queue_push(vec![Value::Str("ish".into()), Value::Int(7)])
                    .is_ok()
            );
        }

        // Worker fon thread'i 3 ishni bajarguncha kutamiz (poll, maksimum ~2s).
        for _ in 0..200 {
            if counter.load(Ordering::SeqCst) == 3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "worker 3 ishning hammasini bajarishi kerak edi"
        );
    }

    // Handler PUSH'dan KEYIN ro'yxatga olinadigan holat: ish navbatda kutib turadi,
    // queue.on kelganda worker uni bajaradi (tartibga bog'liq emas).
    #[test]
    fn push_oldin_on_keyin_kutadi() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let interp = Arc::new(Interp::new());
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        // Avval push — handler hali yo'q, ish navbatda kutadi.
        assert!(
            interp
                .queue_push(vec![Value::Str("kech".into()), Value::Nil])
                .is_ok()
        );

        // Worker birozdan keyin ham bajarmagan bo'lishi kerak (handler yo'q).
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "handler yo'q — bajarilmasin"
        );

        // Endi handler keladi — kutib turgan ish bajarilishi kerak.
        let handler = Value::Native(Arc::new(crate::value::NativeFn {
            name: "inc".into(),
            func: Box::new(move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(Value::Nil)
            }),
        }));
        assert!(
            interp
                .queue_on(vec![Value::Str("kech".into()), handler])
                .is_ok()
        );

        for _ in 0..200 {
            if counter.load(Ordering::SeqCst) == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "handler kelgach kutib turgan ish bajarilishi kerak"
        );
    }

    // Issue #105: drain navbatdagi ishlar TUGASHINI kutadi — qaytgach handler
    // allaqachon ishlagan bo'ladi (poll/race'siz tekshiramiz).
    #[test]
    fn drain_ishlar_tugashini_kutadi() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let interp = Arc::new(Interp::new());
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let handler = Value::Native(Arc::new(crate::value::NativeFn {
            name: "sekin".into(),
            func: Box::new(move |_| {
                // Atayin sekin handler — drain kutmasa counter 3 ga yetmaydi.
                std::thread::sleep(Duration::from_millis(20));
                c.fetch_add(1, Ordering::SeqCst);
                Ok(Value::Nil)
            }),
        }));
        assert!(
            interp
                .queue_on(vec![Value::Str("ish".into()), handler])
                .is_ok()
        );
        for _ in 0..3 {
            assert!(interp.queue_push(vec![Value::Str("ish".into())]).is_ok());
        }

        interp.queue_wait_drain();
        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "drain hamma ish tugashini kutishi kerak"
        );
    }

    // Issue #105: handler'i hech qachon ro'yxatga olinmagan ish drain'ni abadiy
    // ushlab turmaydi — ogohlantirish bilan tashlanadi va navbat bo'shaydi.
    #[test]
    fn drain_handlersiz_ishni_tashlab_yuboradi() {
        let interp = Arc::new(Interp::new());
        assert!(interp.queue_push(vec![Value::Str("yetim".into())]).is_ok());

        // Drain'ni alohida thread'da chaqiramiz — regressiyada (abadiy kutish)
        // test o'zi osilib qolmasin.
        let i2 = interp.clone();
        let h = std::thread::spawn(move || i2.queue_wait_drain());
        for _ in 0..200 {
            if h.is_finished() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            h.is_finished(),
            "drain handler'siz ishda osilib qolmasligi kerak"
        );
        h.join().unwrap();
        assert!(
            interp.queue.inner.lock().queue.is_empty(),
            "tashlangan ish navbatdan chiqishi kerak"
        );
    }

    // Issue #105: handler'siz ish navbat BOSHIDA tursa ham, handler'i bor ish
    // to'silmaydi (eski busy-loop har aylanishda hammasini 50ms kechiktirardi).
    #[test]
    fn handlersiz_ish_boshqalarni_tosmaydi() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let interp = Arc::new(Interp::new());
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        // Avval handler'siz ish — navbat boshini egallaydi.
        assert!(interp.queue_push(vec![Value::Str("yoq".into())]).is_ok());

        let handler = Value::Native(Arc::new(crate::value::NativeFn {
            name: "inc".into(),
            func: Box::new(move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(Value::Nil)
            }),
        }));
        assert!(
            interp
                .queue_on(vec![Value::Str("bor".into()), handler])
                .is_ok()
        );
        assert!(interp.queue_push(vec![Value::Str("bor".into())]).is_ok());

        // Drain "bor" tugashini kutadi, "yoq"ni esa tashlab yuboradi.
        interp.queue_wait_drain();
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "handler'i bor ish handler'siz ish ortida qolib ketmasligi kerak"
        );
    }
}
