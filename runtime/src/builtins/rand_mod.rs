// ---------------- rand (dependency-free LCG) ----------------
use crate::builtins::R;
use crate::builtins::args::*;
use crate::interp::Flow;
use crate::value::Value;

pub(crate) fn rand_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "int" => {
            let a = arg_int(&args, 0, "rand.int")?;
            let b = arg_int(&args, 1, "rand.int")?;
            if b < a {
                return Err(Flow::err("rand.int: upper bound is less than lower bound"));
            }
            // span in i128: at extreme a/b values (e.g. a very negative,
            // b very positive) b - a + 1 does not fit i64 and would overflow.
            let span = (b as i128) - (a as i128) + 1; // [1, 2^64]
            let r = if span > u64::MAX as i128 {
                next_rand() // full i64 range — any u64 is a valid value
            } else {
                next_rand() % (span as u64)
            };
            // The actual result a + r is always within [a, b] — in two's-complement
            // modular arithmetic wrapping_add gives exactly that value (even if the
            // intermediate sum overflows i64).
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
        _ => Err(Flow::err(format!("rand module has no function '{}'", func))),
    }
}

// OS cryptographic CSPRNG (via getrandom, the same source as OsRng in the `auth`
// battery). Previously a thread-local xorshift with seed = system time (nanos) —
// the seed was predictable and two threads opened within the same nanosecond got
// the same sequence. Because `rand.str` is naturally used for token/session-ID
// generation (#97), rand moved entirely to a cryptographically secure source:
// one task = one way — no need to learn a separate "secure rand".
fn next_rand() -> u64 {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    OsRng.next_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    // rand.int stays within bounds (a..=b), including span=1 (a==b).
    #[test]
    fn int_in_range() {
        for _ in 0..1000 {
            let Ok(Value::Int(v)) = rand_module("int", vec![Value::Int(10), Value::Int(20)]) else {
                panic!("rand.int must return an int");
            };
            assert!((10..=20).contains(&v), "out of range: {}", v);
        }
        let Ok(Value::Int(v)) = rand_module("int", vec![Value::Int(7), Value::Int(7)]) else {
            panic!("rand.int must return an int");
        };
        assert_eq!(v, 7); // span=1 -> always a
    }

    // rand.str is the requested length and consists only of alphanumeric chars.
    #[test]
    fn str_len_and_alphabet() {
        let Ok(Value::Str(s)) = rand_module("str", vec![Value::Int(32)]) else {
            panic!("rand.str must return a string");
        };
        assert_eq!(s.chars().count(), 32);
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    // Issue #89: at extreme bounds the span calculation (b - a + 1) overflowed
    // i64 and panicked. Now in i128 — the full i64 range works too.
    #[test]
    fn int_extreme_bounds_no_overflow() {
        for &(a, b) in &[
            (i64::MIN, i64::MAX),     // span = 2^64 (does not fit u64 either)
            (i64::MIN, i64::MIN + 5), // very negative narrow range
            (i64::MAX - 5, i64::MAX), // very positive narrow range
            (-3, i64::MAX),           // span > i64::MAX
        ] {
            for _ in 0..50 {
                let Ok(Value::Int(v)) = rand_module("int", vec![Value::Int(a), Value::Int(b)])
                else {
                    panic!("rand.int must return an int ({}..{})", a, b);
                };
                assert!((a..=b).contains(&v), "out of range: {}", v);
            }
        }
    }

    // Cryptographic source: two consecutive tokens are not identical (unpredictable).
    // With the old xorshift, threads opened within the same nanosecond got the same.
    #[test]
    fn tokens_are_unique() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for _ in 0..100 {
            let Ok(Value::Str(s)) = rand_module("str", vec![Value::Int(24)]) else {
                panic!("rand.str must return a string");
            };
            assert!(seen.insert(s), "duplicate token produced — CSPRNG broken");
        }
    }
}
