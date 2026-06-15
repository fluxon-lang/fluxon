// ---------------- math ----------------
use crate::builtins::R;
use crate::builtins::args::*;
use crate::interp::Flow;
use crate::value::Value;

pub(crate) fn math_module(func: &str, args: Vec<Value>) -> R {
    let x = arg_num(&args, 0, &format!("math.{}", func))?;
    match func {
        "floor" => Ok(Value::Int(x.floor() as i64)),
        "ceil" => Ok(Value::Int(x.ceil() as i64)),
        "abs" => {
            // int in -> int out, flt in -> flt out.
            // i64::MIN.abs() panics (its positive counterpart does not fit) — checked.
            match &args[0] {
                Value::Int(n) => Ok(Value::Int(
                    n.checked_abs().ok_or_else(|| Flow::overflow("math.abs"))?,
                )),
                _ => Ok(Value::Flt(x.abs())),
            }
        }
        "round" => Ok(Value::Int(x.round() as i64)),
        // min/max return the argument itself — int in stays int (same style as
        // abs), and the type is not lost for mixed int/flt either.
        "min" | "max" => {
            // arg_num checks the second argument is a number (x checked the first).
            let y = arg_num(&args, 1, &format!("math.{}", func))?;
            use std::cmp::Ordering;
            // Lossless comparison: casting an int to f64 rounds neighboring values
            // above 2^53 so they compare equal, and the tie rule then returns the
            // wrong side. int/int — in i64, mixed — via cmp_int_flt; only flt/flt
            // stays in f64 (which is exact there).
            let ord = match (&args[0], &args[1]) {
                (Value::Int(a), Value::Int(b)) => a.cmp(b),
                (Value::Int(a), Value::Flt(b)) => cmp_int_flt(*a, *b),
                (Value::Flt(a), Value::Int(b)) => cmp_int_flt(*b, *a).reverse(),
                // NaN is unordered — treat as Equal (same as sorting).
                _ => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            };
            let pick_first = if func == "min" {
                ord != Ordering::Greater
            } else {
                ord != Ordering::Less
            };
            Ok(if pick_first {
                args[0].clone()
            } else {
                args[1].clone()
            })
        }
        "pow" => {
            let y = arg_num(&args, 1, "math.pow")?;
            match (&args[0], &args[1]) {
                // int ^ non-negative int → int (checked: on overflow a Fluxon
                // error, not a panic; an exponent that does not fit i64 is also overflow).
                (Value::Int(a), Value::Int(b)) if *b >= 0 => {
                    let e = u32::try_from(*b).map_err(|_| Flow::overflow("math.pow"))?;
                    Ok(Value::Int(
                        a.checked_pow(e).ok_or_else(|| Flow::overflow("math.pow"))?,
                    ))
                }
                // negative exponent or a flt involved — the result is flt.
                _ => Ok(Value::Flt(x.powf(y))),
            }
        }
        "sqrt" => {
            // The root of a negative number would yield NaN — Fluxon does not
            // expect a NaN value, so an explicit error instead.
            if x < 0.0 {
                return Err(Flow::err(
                    "math.sqrt: cannot take square root of a negative number",
                ));
            }
            Ok(Value::Flt(x.sqrt()))
        }
        _ => Err(Flow::err(format!("math module has no function '{}'", func))),
    }
}

// Compares an i64 with an f64 losslessly: the i64->f64 cast rounds beyond 2^53,
// so the f64 is first compared against the i64 bounds, then the integer and
// fractional parts are compared separately. NaN — Equal (same convention as
// sorting).
fn cmp_int_flt(a: i64, b: f64) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    if b.is_nan() {
        return Equal;
    }
    // i64::MAX as f64 = 2^63 (rounded up) — from that value on, b is greater than
    // any i64. i64::MIN as f64 = -2^63 is represented exactly.
    if b >= i64::MAX as f64 {
        return Less;
    }
    if b < i64::MIN as f64 {
        return Greater;
    }
    // Now b.trunc() fits in i64 and the cast is lossless.
    match a.cmp(&(b.trunc() as i64)) {
        Equal if b.fract() > 0.0 => Less,
        Equal if b.fract() < 0.0 => Greater,
        ord => ord,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Issue #89: i64::MIN.abs() used to panic (its positive counterpart does not
    // fit i64). Now a Fluxon error; ordinary values work as before.
    #[test]
    fn abs_min_is_error_not_panic() {
        let r = math_module("abs", vec![Value::Int(i64::MIN)]);
        let Err(Flow::Error(msg)) = r else {
            panic!("math.abs i64::MIN must return an error");
        };
        assert!(msg.contains("number out of range"), "error text: {}", msg);
        assert!(matches!(
            math_module("abs", vec![Value::Int(-7)]),
            Ok(Value::Int(7))
        ));
    }

    // Issue #128: min/max preserves the argument type — int in, int out, and on
    // mixed int/flt the winner's original type is returned.
    #[test]
    fn min_max_turni_saqlaydi() {
        assert!(matches!(
            math_module("min", vec![Value::Int(3), Value::Int(7)]),
            Ok(Value::Int(3))
        ));
        assert!(matches!(
            math_module("max", vec![Value::Int(3), Value::Int(7)]),
            Ok(Value::Int(7))
        ));
        // mixed: if the flt is smaller it returns flt, if the int is larger it returns int.
        assert!(matches!(
            math_module("min", vec![Value::Int(3), Value::Flt(2.5)]),
            Ok(Value::Flt(v)) if v == 2.5
        ));
        assert!(matches!(
            math_module("max", vec![Value::Int(3), Value::Flt(2.5)]),
            Ok(Value::Int(3))
        ));
        // on equal values the first argument is returned (deterministic).
        assert!(matches!(
            math_module("min", vec![Value::Int(5), Value::Flt(5.0)]),
            Ok(Value::Int(5))
        ));
        // Neighboring ints above 2^53 round to the same f64 — the int/int path
        // must compare exactly in i64 (PR #152 review).
        assert!(matches!(
            math_module("min", vec![Value::Int(i64::MAX), Value::Int(i64::MAX - 1)]),
            Ok(Value::Int(v)) if v == i64::MAX - 1
        ));
        assert!(matches!(
            math_module("max", vec![Value::Int(i64::MAX - 1), Value::Int(i64::MAX)]),
            Ok(Value::Int(v)) if v == i64::MAX
        ));
        // No rounding on mixed int/flt either (PR #152 review):
        // 2^53+1 (int) cast to f64 would come out equal to 2^53.
        let big = (1i64 << 53) + 1; // 9007199254740993
        let big_f = (1i64 << 53) as f64; // 9007199254740992.0
        assert!(matches!(
            math_module("min", vec![Value::Int(big), Value::Flt(big_f)]),
            Ok(Value::Flt(v)) if v == big_f
        ));
        assert!(matches!(
            math_module("max", vec![Value::Flt(big_f), Value::Int(big)]),
            Ok(Value::Int(v)) if v == big
        ));
        // even when the flt is outside the i64 range, the correct side wins.
        assert!(matches!(
            math_module("max", vec![Value::Int(i64::MAX), Value::Flt(1e19)]),
            Ok(Value::Flt(v)) if v == 1e19
        ));
        assert!(matches!(
            math_module("min", vec![Value::Int(i64::MIN), Value::Flt(-1e19)]),
            Ok(Value::Flt(v)) if v == -1e19
        ));
        // case where the fractional part is decisive: 3 < 3.5, -3 > -3.5.
        assert!(matches!(
            math_module("max", vec![Value::Int(3), Value::Flt(3.5)]),
            Ok(Value::Flt(v)) if v == 3.5
        ));
        assert!(matches!(
            math_module("max", vec![Value::Int(-3), Value::Flt(-3.5)]),
            Ok(Value::Int(-3))
        ));
    }

    // Issue #128: int ^ non-negative int -> int (checked), on overflow a Fluxon
    // error not a panic; if the exponent is negative or a flt is involved, flt.
    #[test]
    fn pow_int_flt_va_overflow() {
        assert!(matches!(
            math_module("pow", vec![Value::Int(2), Value::Int(10)]),
            Ok(Value::Int(1024))
        ));
        assert!(matches!(
            math_module("pow", vec![Value::Int(2), Value::Int(-1)]),
            Ok(Value::Flt(v)) if v == 0.5
        ));
        assert!(matches!(
            math_module("pow", vec![Value::Flt(2.0), Value::Int(3)]),
            Ok(Value::Flt(v)) if v == 8.0
        ));
        // 2^63 does not fit i64 — overflow error.
        let r = math_module("pow", vec![Value::Int(2), Value::Int(63)]);
        let Err(Flow::Error(msg)) = r else {
            panic!("math.pow overflow must return an error");
        };
        assert!(msg.contains("number out of range"), "error text: {}", msg);
        // an exponent that does not fit u32 is also overflow (not a panic).
        assert!(math_module("pow", vec![Value::Int(2), Value::Int(u32::MAX as i64 + 1)]).is_err());
    }

    // Issue #128: sqrt always returns flt; a negative input is an explicit error, not NaN.
    #[test]
    fn sqrt_flt_va_manfiy_xato() {
        assert!(matches!(
            math_module("sqrt", vec![Value::Int(9)]),
            Ok(Value::Flt(v)) if v == 3.0
        ));
        assert!(matches!(
            math_module("sqrt", vec![Value::Flt(2.25)]),
            Ok(Value::Flt(v)) if v == 1.5
        ));
        let r = math_module("sqrt", vec![Value::Int(-4)]);
        let Err(Flow::Error(msg)) = r else {
            panic!("math.sqrt of a negative number must return an error");
        };
        assert!(msg.contains("negative"), "error text: {}", msg);
    }
}
