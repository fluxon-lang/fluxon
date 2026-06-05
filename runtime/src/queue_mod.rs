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
//     oldin yozilishi mumkin — worker handler paydo bo'lguncha ishni qaytarib
//     navbat oxiriga qo'yadi va qisqa uxlaydi (busy-spin emas).
//
// Worker — oddiy `std::thread` + `Condvar` (tokio EMAS): handler'lar sinxron
// tree-walking, async kerak emas. Handler `apply` orqali bitta `job` map argumenti
// bilan chaqiriladi; xato worker'ni o'ldirmaydi (cron/ws fire kabi stderr diagnostika).

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

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
    // Worker thread BIR marta yonishi uchun marker (idempotent start).
    started: OnceLock<()>,
}

struct QueueInner {
    queue: VecDeque<Job>,
    handlers: HashMap<String, Value>,
}

impl QueueState {
    pub fn new() -> Self {
        QueueState {
            inner: Mutex::new(QueueInner {
                queue: VecDeque::new(),
                handlers: HashMap::new(),
            }),
            not_empty: Condvar::new(),
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
}

// Worker sikli: navbatdan oldingi ishni oladi, nomiga handler topsa chaqiradi.
// Handler hali yo'q bo'lsa ishni navbat OXIRIGA qaytarib qo'yadi va qisqa uxlaydi
// (busy-spin emas) — handler keyin `queue.on` bilan kelishini kutadi.
fn run_worker(interp: Arc<Interp>) {
    loop {
        // Navbatdan keyingi ishni va (agar bor bo'lsa) handler'ini bitta lock ostida
        // olamiz. Navbat bo'sh bo'lsa Condvar'da uxlaymiz (uyg'otilguncha bloklanadi).
        let (job, handler) = {
            let mut inner = interp.queue.inner.lock();
            while inner.queue.is_empty() {
                interp.queue.not_empty.wait(&mut inner);
            }
            // Front'dagi ishni olamiz; handler'ini darhol shu lock ostida izlaymiz.
            let job = inner.queue.pop_front();
            let Some(job) = job else { continue };
            let handler = inner.handlers.get(&job.name).cloned();
            (job, handler)
        };

        match handler {
            Some(handler) => {
                // Handler'ni SHU thread'da, bitta `job` (payload) argumenti bilan
                // chaqiramiz — FIFO va ketma-ketlik shu bilan kafolatlanadi. Xato
                // worker'ni o'ldirmaydi.
                if let Err(flow) = interp.apply(handler, vec![job.payload]) {
                    eprintln!("queue handler '{}' xatosi: {}", job.name, flow_msg(&flow));
                }
            }
            None => {
                // Handler hali ro'yxatga olinmagan — ishni navbat oxiriga qaytarib,
                // qisqa uxlaymiz. Bu ish push qilingan, lekin handler keyin keladigan
                // holatni qoplaydi (top-level tartibga bog'liq emas).
                interp.queue.inner.lock().queue.push_back(job);
                std::thread::sleep(Duration::from_millis(50));
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
}
