# Flux Frontend qatlami — implementatsiya rejasi

> Manba haqiqat: [`flux-frontend.md`](flux-frontend.md) (spec v2). Bu reja
> runtime'ga frontend qatlamini qo'shish bosqichlarini belgilaydi. Mavjud
> runtime naqshlariga (`http_mod`/`ws_mod`/`reg_mod`/`serve_mod`) sodiq qoladi.
> Har bosqich = mustaqil PR + test + aniq "ishlaydigan ko'rsatkich".

## Eng katta texnik qarorlar (avval hal qilingan)

**Q1 — `view` interp'da nima?** `Value::Fn` + alohida `Interp.views` registri.
`view` = `fn`ning UI varianti (spec) → `apply`/closure/`Parent::Root` bepul keladi.
Yangi `Value` varianti KIRITILMAYDI.

**Q2 — Element daraxti?** Maxsus shaklli map: `{__node:true tag:"div" text:..
children:[..] props:{..}}` — `http_mod`ning `{__resp:true status body}` idiomasi
takrori. Yangi `Value` varianti YO'Q → Send+Sync, barcha `match`/`equals`/`Display`
tegilmaydi. Element konstruktorlari (`div`/`p`/`h1`...) = builtin `Native` funksiyalar.

**Q3 — Render/transpile qayerda?** Bosqichma-bosqich. MVP+dastlab faqat SSR
(`ui_mod::node_to_html` sof Rust funksiya, `body_value_to_response` kabi). Signals
kelganda client = qo'lda yozilgan ~5KB JS (`runtime/src/ui_client.js`, `include_str!`
bilan bundle). Transpiler EMAS — universal client + Phoenix-LiveView server-driven
hidratsiya. JS kodgen yo'q; dastlabki state JSON sifatida HTML'ga embed.

**Q4 — `theme`?** `Interp.theme: Arc<RwLock<BTreeMap>>` (reg_mod state naqshi) →
CSS custom properties (`:root{--primary:..}`), SSR `<head>`ga inject.

**Q5 — `source` → RPC?** Same-file `db.q` → runtime avtomatik `GET /__source/:tag`
endpoint (`http_mod::Route` qayta ishlatiladi); tashqi → `http.get`. `.data/.loading/
.err/.reload` runtime mas'uliyati.

**Q6 — Override?** Yangi mexanizm YO'Q: o'z `view`ingni yoz va chaqir (`reg`
global-almashtirish yo'q — spec halol model). Partial = `cell::`/`fmt::`.

## Bosqichlar

| # | Ishlaydigan ko'rsatkich | Asosiy xavf |
|---|---|---|
| 1 (MVP) | `view` → HTML string (`ui.html (greeting "Ali")`) | element-bola indentation parse |
| 2 | `theme`→CSS, `each`/`if` element render | view-blok element yig'ish |
| 3 | `page` + `ui.serve` SSR (brauzer ochadi) | bir portda HTTP API + UI |
| 4 | `<-` signals, `on:`, `act`, `bind:` reaktiv | server-driven client-state |
| 5 | `source` RPC + `live`/`ui.push` realtime | bir portda HTTP+WS upgrade |
| 6 | `ui.*` bloklar (table/form/...) + override | schema default-by-omission |

### 1-BOSQICH (MVP): `view` + statik element → HTML
**Ko'rsatkich:** `log (ui.html (greeting "Ali"))` → `<h1>Salom Ali</h1><p>...</p>`.

Fayllar:
- `token.rs` — `Tok`: `View Theme Page Source Act` (5 token bir PR'da; MVP faqat `View`).
- `lexer.rs::scan_ident` — 5 kalit so'z jadvali; `parser.rs::keyword_as_name` — member/map-key pozitsiyasida nom.
- `ast.rs` — `Stmt::ViewDecl { name, params, body }`. Element YANGI tugun talab qilmaydi (`h1 "x"` = juxtaposition `Call`).
- `parser.rs` — `parse_view`, `in_view: bool` bayrog'i (`no_app` qatorida). Element-bola: view ichida element-statementdan keyin `Newline Indent` kelsa, blokni parse qilib oxirgi `List` argument (children) sifatida qo'shadi. Backend `fn` semantikasi o'zgarmaydi (faqat `in_view` da).
- `ui_mod.rs` (YANGI) — core teglar (`div p h1 h2 h3 span btn img input a ul li form badge`) builtin `Native`, `{__node}` quradi. `node_to_html(&Value)->String` sof funksiya. `ui.html` dispatch.
- `interp.rs` — `ViewDecl` hoisting (`Value::Fn` + `views` registri, `Parent::Root`); `Interp.views: Arc<RwLock<HashSet<String>>>`; `eval_call`da `if modname=="ui"` dispatch.
- `main.rs` — `mod ui_mod;` + integratsiya testi.

Test: `node_to_html` unit (escape/props→class/nested); `.fx` e2e `view_static_render`.

### 2-BOSQICH: `theme` + semantik props → CSS, `each`/`if` render
- `ast.rs` `Stmt::Theme { tokens }` (space-separated, `tbl` naqshi); `parser::parse_theme`.
- `interp.rs` Theme → `Interp.theme`. `ui_mod::theme_to_css`, `ui_base.css` (`include_str!`), props→class (`flux-primary flux-pad-4`).
- View-blok element yig'ish: `exec_view_block` — har `Stmt::Expr` natijasi `{__node}` bo'lsa children'ga. `each`/`if` (mavjud) view ichida ro'yxat/shox quradi.

### 3-BOSQICH: `page` routing + `ui.serve` (SSR)
- `ast.rs` `Stmt::Page { pattern, handler }` (`-> view` / `\params -> ...`); `parser::parse_page` (`http.on` naqshi).
- `ui_mod` `Interp.pages` (`http_mod::Route` qayta ishlatish). `serve_mod::PendingServer::Ui{port}`, `run_pending`da `ui_mod::serve_loop`.
- Bir port: `/api/*` → http routes, qolgani → page SSR. `http_mod::match_route`/`value_to_response` ni `pub(crate)` qilib qayta ishlatish.

### 4-BOSQICH: Fine-grained signals + hidratsiya (`<-` reaktiv)
- `runtime/src/ui_client.js` (YANGI resurs) — qo'lda ~5KB signal+patcher+event delegation, `/__client.js` da serve.
- SSR reaktiv qismlarga `data-fx-bind`/`data-fx-on` marker + `window.__fx_state` JSON embed.
- Server-driven: `on:` event → server (WS/HTTP) → view qayta render → DOM patch (LiveView). Client thin.
- `ast.rs` `Stmt::Act { name, body }` (view ichidagi handler, state ko'radi → closure capture bepul). `bind:` → `value+on:input` (props parse).

### 5-BOSQICH: `source` RPC + `live` realtime
- `ast.rs` `Expr::Source { live, tag, query }`; tag = bind nomi. `parser::parse_source` (`source [live] db.q ...` / `http.get` / dinamik `source if cond ...`).
- Same-file `db.q` → `/__source/:tag` endpoint avto. `.data/.loading/.err/.reload` client obyekti.
- `ui.invalidate :tag` (lokal reload), `ui.push :tag` (= `ws.room.send` tag-kanal), `ui.on :tag` (xom subscribe).
- Bir portda HTTP+WS upgrade (`Connection: Upgrade` qo'lda; `ws_mod::handle_conn`/`room` qayta ishlatiladi). **Alohida sub-PR bo'lishi mumkin.**

### 6-BOSQICH: `ui.*` battery + override
- `ui_dispatch`: `table/form/stat/chart/modal/input/select/search/badge/btn/spinner/error/shell` — har biri `{__node}` daraxti, schema'dan default-by-omission (`Interp.schema`).
- Override: mexanizm yo'q (o'z `view`ingni yoz+chaqir). `ui.table` da `cell::`/`fmt::` per-column.

## Doimiy invariantlar (har bosqichda)
- **Send+Sync:** yangi `Value` varianti yo'q (`{__node}`/`{__resp}` map). `Interp` yangi state `Arc<...>` (reg_mod/ws_mod naqshi).
- **`Parent::Root`:** view'lar top-level `fn` kabi — tegilmaydi.
- **`freeze_globals`:** `ui.serve` server — mavjud muzlatish.
- **Test majburiy:** `main.rs::mod tests` (.fx e2e) + modul-ichi sof Rust unit.
- **Canonical/kam token:** element = juxtaposition-call; `on:`/`bind:` = props; override = view+chaqiruv (yangi keyword yo'q).
