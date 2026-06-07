# Flux Frontend — language spec (for AI)

Flux frontend: UI in the SAME `.fx` file as backend. Reactive component tree.
One task = one way. Few tokens. Batteries-included (`ui.*`). No JS framework,
no Tailwind in source, no JSX, no closing tags. Read once, write correct Flux UI.

Reuses backend Flux as-is: `<-` (reactive state), `each` (list render),
`if/elif/else` (conditional render), `match`, space-args (component call),
indentation (element tree), `db.q`/`http`/`ai` (data).
NEW keywords: `view` `theme` `page` `source` `act`.

`match`/`if` are EXPRESSIONS (return the matched arm) — usable in `=`, props, args:
```flux
kind = match st (:new -> :info  :done -> :ok  _ -> :muted)
badge st {kind:(if st == :done :ok else :muted)}
```

## view — component
A `view` is the UI variant of `fn`: args = props, body = element tree.
```flux
view greeting name
  h1 "Salom $name"
  p "xush kelibsiz"
```
Call = space-args, same as a backend `fn`: `greeting "Ali"`.
Default-valued prop: `name:default`.
```flux
view stat label value icon:nil
  div {kind::card pad:4}
    p value {size::xl bold:true}
    p label {kind::muted}
    if icon
      span icon
stat "Daromad" "$1200" icon::cash      # call
```

## Element — `tag content {props}`
Canonical element (NOT HTML, NO closing tag): tag name, then text/children,
then optional `{props}` map. Children = indentation (like `tbl`).
```flux
btn "Saqlash" {on:save kind::primary}    # one line: text + props
div {kind::panel gap:3}                   # children indented
  h1 "Mahsulotlar"
  p "${items.len} ta" {kind::muted}
```
Core tags: `div p h1 h2 h3 span btn img input a ul li form badge`.

Props are SEMANTIC (NOT CSS classes, NOT Tailwind):
`kind::primary` (`:primary :ok :warn :danger :info :muted :ghost :card :panel :row`)
· `pad:N gap:N mb:N mt:N ml:N w:N` · `size::xl size::sm` · `bold:true round:true`
· `grid:N flex:true hover:true`. Exact colors/fonts live in `theme`.

## Reactive state — `<-` (NO new symbol)
The backend mutable bind IS the UI signal. On change, only the bound DOM
re-renders (fine-grained, no virtual-DOM). NO `state` block, NO `watch`.
Derived value = `=` (computed, memoized).
```flux
view counter
  n <- 0                    # reactive state
  doubled = n * 2           # computed (recomputes when n changes)
  p "Soni: $n (x2: $doubled)"
  btn "+1" {on:\-> n <- n + 1}
```

## Events — `on:` in props
No new event syntax. `on:` = element-default event (btn→click, form→submit,
input→change). Value = fn-value `{on:save}` or lambda `{on:\-> ...}`.
Explicit event: `on::click` / `on::input`. Lambda arg `e` (`e.value`).
```flux
btn "Saqlash" {on:save}                # fn value (click)
btn "+1" {on:\-> n <- n + 1}           # lambda (click)
input {on::input \e -> q <- e.value}   # explicit event
form {on:submit_handler}               # form → submit
```

## bind: — two-way binding
`input value + on:input` collapsed to one word. `bind:x` = state name.
```flux
q <- ""
input {bind:q placeholder:"Qidirish..."}   # = value:q + on:input(\e -> q <- e.value)
```

## act — handlers & state scope
`<-` state is LOCAL to its `view`. An outside top-level `fn` CANNOT mutate it
(no global mutation). Write the handler inline as a lambda, OR name a multi-line
handler with `act` INSIDE the view (it closes over the view's state).
```flux
view menu_page
  open <- false
  edit <- nil
  act open_new            # multi-line handler, sees `open`/`edit`
    edit <- nil
    open <- true
  btn "+ Yangi" {on:open_new}
  ui.modal {open:open}
    ui.form edit {on:save}    # `save` = plain fn (data in, http out)
```
A plain top-level `fn` is for data/IO (http/db): takes args, returns, NEVER
touches view state. After a server mutation it calls `ui.invalidate`/`ui.push`.

## each / if — list & conditional render (reused)
No new constructs. `each` renders a list (`key:` optional, for diff).
`if/elif/else` for conditional render. NO postfix if.
```flux
each p in items key:p.id
  div {kind::row}
    b p.name
    if p.stock == 0
      badge "Tugadi" {kind::danger}
    else
      span "${p.stock} dona"
```

## source — reactive data (NO glue code)
`source` wraps backend `db.q`/`db.one`/`http`/`ai`. Auto loading/error/refetch.
Bind into a `<-` state. Same-file `db.q` → runtime auto-generates the endpoint;
external → `http.get`. NO fetch/useEffect/parse — runtime owns it.
```flux
view products_page
  items <- source db.q "select * from products order by ts desc"
  q     <- ""
  shown = items.data.filter \p -> str.has (str.low p.name) (str.low q)

  input {bind:q placeholder:"Qidirish..."}
  if items.loading
    ui.spinner
  elif items.err
    ui.error items.err
  else
    ui.table shown {cols:[:name :price :stock]}
```
Fields: `items.data` `items.loading` `items.err` `items.reload()`.
A source's TAG is its bind name: `items <- source ...` registers tag `:items`.
`ui.invalidate :items` / `ui.push :items` / `ui.on :items` all refer to it.
Multi-word names work: `staff_list <- source ...` → `:staff_list`.
After a mutation, refresh by tag: `ui.invalidate :items` (re-runs the source).
```flux
fn save_product d
  http.post "/api/products" d
  ui.invalidate :items        # source reloads → table updates
```
External data: `source http.get "/url"` → `.data` is the parsed body directly
(not `{status body}`).

### Dynamic source — refetch on a reactive arg
Bind a source to reactive state — it refetches when that state changes (no
imperative fetch). Read loaded data with derived `=` instead of copying into
state on load (NO `watch`/`effect`):
```flux
view orders_page
  sel    <- nil                                            # selected id
  detail <- source if sel db.one "select * from orders where id=$1" [sel]
  act show \r -> sel <- r.id                               # click → detail refetches
  name = detail.data.cust ?? ""                            # derived, recomputes on load
```

### Realtime source — `live`
A `source` marked `live` auto-subscribes to the server WS channel named by its
tag. When the server calls `ui.push :tag`, every connected client's matching
source reloads — NO client WS code. `ui.push` is the broadcast twin of the
local `ui.invalidate`. `ui.serve` owns the WS channel (same port — no separate
`ws.serve`).
```flux
orders <- source live db.q "select * from orders order by ts desc"
# server side, after a mutation:
fn save_order d
  db.ins "orders" d
  ui.push :orders           # ALL clients' :orders source reloads (via WS)
```
Raw WS messages (no source) — `ui.on :tag` inside a view:
```flux
ui.on :orders \msg -> log msg.event
```

## theme — config layer
Global design tokens, space-separated (like `tbl`). Customers tune colors/font
here WITHOUT touching components.
```flux
theme
  primary "#e84d8a"
  accent  "#9b5de5"
  radius  :lg              # :sm :md :lg :xl
  font    "Inter"
  mode    :light           # :light :dark :auto
```
Tokens compile to CSS custom properties; semantic props read from them.

## ui.* — default battery (shadcn-style, `use ui`)
Ready-made, designed, accessible blocks. Install-free (batteries). Each obeys
`theme`. Default-by-omission: omit args → inferred from data/schema.
Blocks: `ui.shell` (sidebar+header layout) · `ui.table` · `ui.form` · `ui.stat`
· `ui.chart` · `ui.modal` · `ui.input` · `ui.select` · `ui.search` · `ui.badge`
· `ui.btn` · `ui.spinner` · `ui.error`.
```flux
ui.stat "Daromad" "${s.revenue/100}$" {icon::cash kind::primary}
ui.table products {cols:[:name :price :stock]
  fmt::price \v -> "${v/100}$"                     # format a column
  cell::stock \r -> badge r.stock {kind:(if r.stock==0 :danger else :ok)}  # custom cell
  actions:[{icon::edit on:\r -> edit_row r}]}      # row actions
ui.modal {open:open title:"Yangi mahsulot"}
  ui.form edit {on:save_product fields:[
    {name::name  label:"Nomi"  kind::text req:true}
    {name::price label:"Narx"  kind::money}
    {name::cat   label:"Tur"   kind::select opts:[:atirgul :lola :buket]}
  ]}
```
`ui.chart` (props mirror `ui.table`):
```flux
ui.chart data {kind::line x::day y::rev fmt::rev \v -> "${v/100}$"}
# kind:: :line :bar :area · x::/y:: = field syms · fmt:: per series
```
`ui.select` standalone, with optional labels:
```flux
ui.select {bind:cat opts:[:all :main :drink]}                  # symbols → label from name
ui.select {bind:cat opts:[[:all "Barchasi"] [:main "Asosiy"]]}  # [val label] pairs
```
`ui.form` field `kind::` — full list:
`:text :money :number :select :bool :color :date :textarea`.
Repeating field (N rows, e.g. order lines) — `kind::list` with sub-`fields`
(value = array of maps):
```flux
{name::items label:"Taomlar" kind::list fields:[
  {name::item kind::select opts:menu} {name::qty kind::number}]}
```

## Three levels: default → config → override
The SAME call shape at every level — moving from default to override only
changes the name, not the call site.
```flux
ui.table products                                   # DEFAULT (cols from schema)
ui.table products {cols:[:name :price] search::name}  # CONFIG (tune via props)

# OVERRIDE — declare a `view` with your own name; call it instead.
view my_table rows
  div {kind::panel}
    each r in rows key:r.id
      div {kind::row hover:true}
        img r.photo {w:8 round:true}
        b r.name
        span "${r.price/100}$" {ml::auto}
my_table products                                   # same call shape
```
Override = write a `view` with your own name and call it instead of `ui.X` —
no new keyword, no registration. For a small tweak, don't rewrite the whole
block: use `cell::`/`fmt::` (per-column, shown above) for partial override.

## page — routing (declarative)
UI variant of `http.on`, URL = page (not file-system). `:param` splits like
backend routes (literal > param). `nav` = SPA link (no reload).
```flux
page "/" -> dashboard
page "/products" -> products_page
page "/orders/:id" \params -> order_page params.id
nav "/products" "Mahsulotlar"
```

## ui.serve — one entry point
HTTP API + UI client bundle + WS (`live`/`ui.push`/`ui.on`) on ONE port, ONE
event-loop. No separate `ws.serve` for UI realtime.
```flux
ui.serve app 3000
```

## Full example (frontend + backend, one file)
```flux
use http db ui

tbl products
  id    serial pk
  name  str
  price money
  stock int
  cat   sym
  ts    now

http.on :get  "/api/products"     \req -> rep 200 (db.q "select * from products order by ts desc")
http.on :post "/api/products"     \req -> rep 201 (db.ins "products" req.body)
http.on :del  "/api/products/:id" \req -> rep 200 (db.del "products" {id:req.params.id})

theme
  primary "#e84d8a"
  radius  :lg
  mode    :light

view app
  ui.shell {brand:"Gulzor" nav:[{to:"/" icon::box label:"Mahsulotlar"}]}
    page "/" -> products_page

view products_page
  items <- source live db.q "select * from products order by ts desc"   # realtime
  q     <- ""
  open  <- false
  shown = items.data.filter \p -> str.has (str.low p.name) (str.low q)
  act add_new                          # view-local handler (sees `open`)
    open <- true

  div {flex:true gap:3 mb:4}
    h1 "Mahsulotlar"
    ui.search {bind:q placeholder:"Qidirish..."}
    btn "+ Yangi" {on:add_new kind::primary ml::auto}

  ui.table shown {cols:[:name :cat :price :stock]
    fmt::price \v -> "${v/100}$"
    cell::stock \r -> badge r.stock {kind:(if r.stock==0 :danger else :ok)}
    actions:[{icon::trash on:\r -> del_product r.id confirm:"O'chirilsinmi?"}]}

  ui.modal {open:open title:"Yangi mahsulot"}
    ui.form nil {on:save_product fields:[
      {name::name  label:"Nomi" kind::text req:true}
      {name::price label:"Narx" kind::money}
      {name::cat   label:"Tur"  kind::select opts:[:atirgul :lola :buket]}
    ]}

fn save_product d                      # plain fn: data/IO only
  http.post "/api/products" d
  ui.push :items                       # ALL clients reload (live)
  ui.close

fn del_product id
  http.del "/api/products/$id"
  ui.push :items

ui.serve app 3000
```
