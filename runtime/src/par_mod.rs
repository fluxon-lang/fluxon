// par — til-darajasidagi parallel fan-out primitivi (issue #137).
//
// `par [\-> ai.ask p1  \-> http.get u2  \-> db.one "..." [id]]` lambdalar
// ro'yxatini oladi, HAR BIRINI alohida thread'da chaqiradi va hammasi tugaguncha
// kutadi. Natija — kirish tartibidagi ro'yxat; har element `{ok: qiymat}` (lambda
// muvaffaqiyatli) yoki `{err: xabar}` (lambda `fail`/xato ko'targan). Bitta lambda
// muvaffaqiyatsiz bo'lsa qolganlari to'xtamaydi (qisman muvaffaqiyat: 3 API'dan 2
// tasi ishladi — chaqiruvchi har natijani tekshiradi).
//
// Nega thread (tokio emas): `par` blocking kontekstda (top-level kod yoki HTTP/WS
// handler ichida, ular allaqachon o'z thread'ida) chaqiriladi. `std::thread` +
// `join` eng sodda va to'g'ri model — handler kabi sinxron yo'lga mos. `Value` va
// `Flow` Send (invariant), `Arc<Interp>` thread'lar orasida ulashiladi (cron/queue
// kabi). Lambda closure'lari `Parent::Scope(env)` ni ushlaydi — `Arc<RwLock<Scope>>`,
// shuning uchun parallel o'qish (lookup) xavfsiz; `<-` yozuvlari RwLock write bilan
// ketma-ketlanadi.

use std::sync::Arc;

use crate::interp::{Flow, Interp};
use crate::value::Value;

impl Interp {
    // par [\-> ... \-> ...] — yagona argument: lambdalar ro'yxati.
    pub fn par_run(self: &Arc<Self>, args: Vec<Value>) -> Result<Value, Flow> {
        let mut it = args.into_iter();
        let list = match it.next() {
            Some(Value::List(xs)) => xs,
            Some(other) => {
                return Err(Flow::err(format!(
                    "par: argument lambdalar ro'yxati bo'lishi kerak, {} berildi",
                    other.type_name()
                )));
            }
            None => return Err(Flow::err("par: lambdalar ro'yxati argumenti kerak")),
        };
        if it.next().is_some() {
            return Err(Flow::err(
                "par: faqat bitta argument (lambdalar ro'yxati) kutiladi",
            ));
        }
        // Har element chaqirib bo'ladigan funksiya (0-arity lambda yoki fn value)
        // ekanini OLDINDAN tekshiramiz — thread ochilmasdan aniq xato beramiz.
        for (i, v) in list.iter().enumerate() {
            if !matches!(v, Value::Fn(_) | Value::Native(_)) {
                return Err(Flow::err(format!(
                    "par: {}-element funksiya bo'lishi kerak (\\-> ...), {} berildi",
                    i,
                    v.type_name()
                )));
            }
        }

        // Bo'sh ro'yxat — bo'sh natija (thread ochmaymiz).
        if list.is_empty() {
            return Ok(Value::List(Vec::new()));
        }

        // db.tx ichidan par CHAQIRIB BO'LMAYDI: tranzaksiya joriy thread'ning
        // `CURRENT_TX` TLS'ida turadi va yangi thread'lar uni meros qilmaydi —
        // par lambda'lari jim ravishda tranzaksiyadan TASHQARIDA (global DB,
        // commit qilinmagan o'zgarishlarni ko'rmay) ishlardi, db.tx atomiklik/
        // read-your-writes semantikasini buzib. SQLite connection thread-safe
        // bo'lmagani uchun tx'ni ulashib ham bo'lmaydi. Jim noto'g'ri ishlash
        // o'rniga aniq xato beramiz (issue #137 PR review, P1).
        if crate::db_mod::tx_active() {
            return Err(Flow::err(
                "par: db.tx ichida ishlatib bo'lmaydi (tranzaksiya thread'lar orasida ulashilmaydi); tx tashqarisida chaqiring",
            ));
        }

        // Har lambdani alohida thread'da chaqiramiz. Thread `Arc<Interp>` klonini
        // ushlaydi (zaif emas — chaqiruv davomida tirik turishi kafolatlangan).
        let handles: Vec<_> = list
            .into_iter()
            .map(|f| {
                let interp = self.clone();
                std::thread::spawn(move || interp.apply(f, Vec::new()))
            })
            .collect();

        // Kirish tartibida join qilamiz — natija ro'yxati lambdalar tartibiga mos.
        let mut out = Vec::with_capacity(handles.len());
        for h in handles {
            let cell = match h.join() {
                // Lambda muvaffaqiyatli yoki Flow xato qaytardi.
                Ok(res) => flow_to_cell(res),
                // Thread panic qildi (apply ichida kutilmagan panic) — xato cell.
                Err(_) => err_cell("par: ichki thread panic qildi"),
            };
            out.push(cell);
        }
        Ok(Value::List(out))
    }
}

// `apply` natijasini `{ok:...}` yoki `{err:...}` map'ga aylantiradi. `apply`
// `Flow::Return`ni allaqachon `Ok`ga normallashtirgan; shu yerga faqat haqiqiy
// natija yoki `Fail`/`Error`/`Skip`/`Stop` keladi. `skip`/`stop` lambda ichida
// ma'nosiz (loop yo'q) — ularni ham xato deb belgilaymiz.
fn flow_to_cell(res: Result<Value, Flow>) -> Value {
    match res {
        Ok(v) => ok_cell(v),
        Err(Flow::Fail { message, .. }) | Err(Flow::Error(message)) => err_cell(message),
        Err(Flow::Skip) => err_cell("par: lambda ichida `skip` (loop yo'q)"),
        Err(Flow::Stop) => err_cell("par: lambda ichida `stop` (loop yo'q)"),
        // Return apply tomonidan Ok'ga aylantirilgan — bu shoxga yetmaydi, lekin
        // to'liqlik uchun qiymatini ok cell qilamiz.
        Err(Flow::Return(v)) => ok_cell(v),
    }
}

fn ok_cell(v: Value) -> Value {
    let mut m = std::collections::BTreeMap::new();
    m.insert("ok".to_string(), v);
    Value::Map(m)
}

fn err_cell(msg: impl Into<String>) -> Value {
    let mut m = std::collections::BTreeMap::new();
    m.insert("err".to_string(), Value::Str(msg.into()));
    Value::Map(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    // flow_to_cell: Ok -> {ok:v}, Fail/Error -> {err:msg}, skip/stop -> {err:...}.
    fn cell_kind(v: &Value) -> &'static str {
        match v {
            Value::Map(m) if m.contains_key("ok") => "ok",
            Value::Map(m) if m.contains_key("err") => "err",
            _ => "boshqa",
        }
    }

    #[test]
    fn ok_natija_ok_cell() {
        assert_eq!(cell_kind(&flow_to_cell(Ok(Value::Int(5)))), "ok");
    }

    #[test]
    fn fail_xato_err_cell() {
        let c = flow_to_cell(Err(Flow::Fail {
            status: Some(422),
            message: "yo'q".into(),
        }));
        assert_eq!(cell_kind(&c), "err");
        if let Value::Map(m) = &c {
            assert!(matches!(m.get("err"), Some(Value::Str(s)) if s == "yo'q"));
        }
    }

    #[test]
    fn runtime_xato_err_cell() {
        assert_eq!(cell_kind(&flow_to_cell(Err(Flow::err("buzildi")))), "err");
    }

    // skip/stop lambda ichida loop yo'q — xato cell.
    #[test]
    fn skip_stop_err_cell() {
        assert_eq!(cell_kind(&flow_to_cell(Err(Flow::Skip))), "err");
        assert_eq!(cell_kind(&flow_to_cell(Err(Flow::Stop))), "err");
    }
}
