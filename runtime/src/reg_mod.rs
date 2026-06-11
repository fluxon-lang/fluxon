// Fluxon reg battery — funksiya registri (dinamik dispatch).
//
// Funksiyani STRING nomi bilan saqlash/chaqirish imkonini beradi. Asosiy
// foydalanish — AI agent tool-loop'lari: model "qaysi tool'ni" string nomi
// bilan tanlaydi, kod esa `reg.call name args` orqali uni bajaradi (`match`-switch
// EMAS — tool'lar runtime'da qo'shiladi).
//
// Til API (docs/fluxon-agent.md):
//   reg.add "calc" \args -> args.a + args.b   # nom bilan ro'yxatga olish
//   out = reg.call "calc" {a:2 b:3}           # nom bilan chaqirish -> 5
//   reg.has "calc"                            # bool
//   reg.names                                 # ro'yxatdagi nomlar (list)
//
// Saqlangan qiymat — Value::Fn (closure) yoki Value::Native. Closure top-level'da
// e'lon qilingani uchun `Parent::Root` ushlaydi -> `http.serve`/`ws.serve`
// muzlatgandan keyin ham (boshqa thread'da) muzlatilgan global'lardan to'g'ri
// o'qiydi. Value: Send+Sync bo'lgani uchun registr thread'lar aro xavfsiz.

use std::collections::HashMap;

use parking_lot::Mutex;

use crate::interp::{Flow, Interp};
use crate::value::Value;

// reg battery holati — jarayonga bitta (Interp ichida Arc). Top-level kod
// `reg.add` bilan to'ldiradi; `reg.call` istalgan thread'dan (http/ws handler
// ichidan ham) o'qiydi. http `routes`/ws `WsState` bilan bir xil model.
pub struct RegState {
    // nom -> funksiya (Value::Fn yoki Value::Native).
    fns: Mutex<HashMap<String, Value>>,
}

impl RegState {
    pub fn new() -> Self {
        RegState {
            fns: Mutex::new(HashMap::new()),
        }
    }
}

impl Interp {
    // reg.* dispatch — `eval_call` Field{Ident("reg"), name}'ni shu yerga
    // yo'naltiradi. `reg.names` argumentsiz ham keladi (Field, Call emas) —
    // u ham shu funksiyaga (bo'sh argv bilan) tushadi.
    pub fn reg_dispatch(&self, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            // reg.add "nom" fn — funksiyani nom bilan saqlaydi. Qaytadi: nil.
            // Mavjud nom ustiga yozadi (qayta ro'yxatga olish — tool yangilash).
            "add" => {
                let mut it = args.into_iter();
                let name = match it.next() {
                    Some(Value::Str(s)) => s,
                    Some(other) => {
                        return Err(Flow::err(format!(
                            "reg.add: birinchi argument nom (str) bo'lishi kerak, {} berildi",
                            other.type_name()
                        )));
                    }
                    None => return Err(Flow::err("reg.add: nom va funksiya argumentlari kerak")),
                };
                let f = match it.next() {
                    Some(f @ (Value::Fn(_) | Value::Native(_))) => f,
                    Some(other) => {
                        return Err(Flow::err(format!(
                            "reg.add: ikkinchi argument funksiya bo'lishi kerak, {} berildi",
                            other.type_name()
                        )));
                    }
                    None => return Err(Flow::err("reg.add: funksiya argumenti kerak")),
                };
                self.reg.fns.lock().insert(name, f);
                Ok(Value::Nil)
            }
            // reg.call "nom" args -> funksiyani nom bilan chaqiradi va natijasini
            // qaytaradi. Nom topilmasa fail. Funksiyani lock TASHQARISIDA chaqiramiz
            // (apply uzoq ishlashi va reg'ga qayta kirishi mumkin -> deadlock'siz).
            "call" => {
                let mut it = args.into_iter();
                let name = match it.next() {
                    Some(Value::Str(s)) => s,
                    Some(other) => {
                        return Err(Flow::err(format!(
                            "reg.call: birinchi argument nom (str) bo'lishi kerak, {} berildi",
                            other.type_name()
                        )));
                    }
                    None => return Err(Flow::err("reg.call: nom argumenti kerak")),
                };
                let f = match self.reg.fns.lock().get(&name) {
                    Some(f) => f.clone(),
                    None => return Err(Flow::err(format!("reg.call: '{}' ro'yxatda yo'q", name))),
                };
                let rest: Vec<Value> = it.collect();
                self.apply(f, rest)
            }
            // reg.has "nom" -> bool. Nom ro'yxatda bormi.
            "has" => {
                let name = match args.into_iter().next() {
                    Some(Value::Str(s)) => s,
                    Some(other) => {
                        return Err(Flow::err(format!(
                            "reg.has: argument nom (str) bo'lishi kerak, {} berildi",
                            other.type_name()
                        )));
                    }
                    None => return Err(Flow::err("reg.has: nom argumenti kerak")),
                };
                Ok(Value::Bool(self.reg.fns.lock().contains_key(&name)))
            }
            // reg.names -> ro'yxatdagi nomlar (list, alifbo tartibida — barqaror chiqish).
            "names" => {
                let mut names: Vec<String> = self.reg.fns.lock().keys().cloned().collect();
                names.sort();
                Ok(Value::List(names.into_iter().map(Value::Str).collect()))
            }
            other => Err(Flow::err(format!("reg.{} funksiyasi yo'q", other))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::NativeFn;
    use std::sync::Arc;

    // Value/Flow Debug derive qilmaydi (unwrap/{:?} ishlatib bo'lmaydi) —
    // natijani qo'lda match qilamiz. Quyidagi yordamchilar shuni ixchamlashtiradi.

    // Test yordamchisi: berilgan natijani qaytaradigan native funksiya.
    fn native_const(name: &str, v: i64) -> Value {
        let name = name.to_string();
        Value::Native(Arc::new(NativeFn {
            name,
            func: Box::new(move |_args| Ok(Value::Int(v))),
        }))
    }

    // Flow xato matnini ajratib oladi (Ok bo'lsa panic).
    fn err_msg(r: Result<Value, Flow>) -> String {
        match r {
            Err(Flow::Error(m)) | Err(Flow::Fail { message: m, .. }) => m,
            Err(_) => panic!("kutilmagan oqim turi"),
            Ok(_) => panic!("xato kutilgandi, Ok keldi"),
        }
    }

    #[test]
    fn add_then_has_and_call() {
        let it = Interp::new();
        assert!(
            it.reg_dispatch("add", vec![Value::Str("x".into()), native_const("x", 42)])
                .is_ok()
        );
        // has -> true
        assert!(matches!(
            it.reg_dispatch("has", vec![Value::Str("x".into())]),
            Ok(Value::Bool(true))
        ));
        // call -> 42
        assert!(matches!(
            it.reg_dispatch("call", vec![Value::Str("x".into())]),
            Ok(Value::Int(42))
        ));
    }

    #[test]
    fn has_unknown_is_false() {
        let it = Interp::new();
        assert!(matches!(
            it.reg_dispatch("has", vec![Value::Str("yoq".into())]),
            Ok(Value::Bool(false))
        ));
    }

    #[test]
    fn call_unknown_errors() {
        let it = Interp::new();
        let m = err_msg(it.reg_dispatch("call", vec![Value::Str("yoq".into())]));
        assert!(m.contains("ro'yxatda yo'q"), "matn: {m}");
    }

    #[test]
    fn names_sorted() {
        let it = Interp::new();
        assert!(
            it.reg_dispatch("add", vec![Value::Str("b".into()), native_const("b", 1)])
                .is_ok()
        );
        assert!(
            it.reg_dispatch("add", vec![Value::Str("a".into()), native_const("a", 2)])
                .is_ok()
        );
        match it.reg_dispatch("names", vec![]) {
            Ok(Value::List(xs)) => {
                let got: Vec<String> = xs
                    .iter()
                    .map(|v| match v {
                        Value::Str(s) => s.clone(),
                        _ => panic!("nom str emas"),
                    })
                    .collect();
                assert_eq!(got, vec!["a".to_string(), "b".to_string()]);
            }
            _ => panic!("names list qaytarmadi"),
        }
    }

    #[test]
    fn add_rejects_non_fn() {
        let it = Interp::new();
        let m = err_msg(it.reg_dispatch("add", vec![Value::Str("x".into()), Value::Int(5)]));
        assert!(m.contains("funksiya"), "matn: {m}");
    }
}
