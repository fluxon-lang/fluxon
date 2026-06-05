# Flux Runtime

Flux tilining interpretatori (Rust, tree-walking). Hozircha **til yadrosi**
ishlaydi — batteries (`http`, `db`, `ai`, ...) hali qo'shilmagan.

## Qurish va ishga tushirish

```sh
cargo build --release
cargo run -- run examples/demo.fx
# yoki
./target/release/flux run examples/demo.fx
```

## Hozir nima ishlaydi

Til yadrosining to'liq qismi:

- **Tiplar:** int, flt, str, bool, nil, sym, list, map
- **Bindings:** `=` (o'zgarmas), `<-` (o'zgaruvchan)
- **Funksiyalar:** `fn`, bir qatorli `->`, lambda `\x ->`, closure, `ret`, oxirgi-ifoda qaytarish, rekursiya
- **Boshqaruv:** `if`/`elif`/`else`, `each` (list/map/range/str), `skip`/`stop`, `match` (symbol/son/`_`)
- **Operatorlar:** arifmetik (`+ - * / %`), taqqoslash, mantiqiy (`& | !`), `??`, `|>`, `..`, member/indeks (`.` `[]`)
- **String interpolatsiya:** `"$x"`, `"${expr}"`
- **List metodlari:** `len push has filter map reduce slice join`
- **Map metodlari:** `len has keys vals set del` + spread `{...m}` + dinamik kalit `{[k]:v}`
- **Modullar:** `str` (len up low slice split has int str), `math` (floor ceil abs round), `rand` (int str), `json` (enc dec)
- **`log`** — stderr'ga chiqarish
- **Xatolar:** `fail [status] "..."`, `!` (propagate o'tkazgich)

`use` va `tbl` parse qilinadi, lekin yadroda e'tiborsiz qoldiriladi (batteries
fazasi uchun).

## Arxitektura

```
src/
  token.rs    — token tiplari (+ INDENT/DEDENT, string bo'laklari)
  lexer.rs    — manba -> tokenlar; indentatsiya -> INDENT/DEDENT
  ast.rs      — AST tugunlari
  parser.rs   — tokenlar -> AST (precedence climbing + qavssiz chaqirish)
  value.rs    — runtime qiymatlar
  interp.rs   — AST'ni walk qilib bajaruvchi (scope, control flow, chaqiruv)
  builtins.rs — yadro funksiyalari (modullar + qiymat metodlari)
  main.rs     — CLI + integratsiya testlari
```

Frontend (lexer/parser/AST) kelajakda bytecode VM'ga ham qayta ishlatilishi
mumkin.

## Testlar

```sh
cargo test
```

7 ta integratsiya testi `src/main.rs` ichida (fib, list metodlari, map, mutable
each, match, str/modullar, pipe/coalesce).

## Keyingi qadam

Batteries: `http.serve`/`http.on`, in-memory `db`, keyin Postgres, `ai`, `ws`,
`cron`, `queue`.
