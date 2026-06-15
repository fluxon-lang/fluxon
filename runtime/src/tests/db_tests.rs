use super::*;

#[test]
fn db_ins_sym_json_roundtrip() {
    // ins returns the generated id; sym Str<->Sym; json map round-trip.
    with_db_test("ins_sym_json", || {
        run(r#"
use db
tbl tickets
  id       serial pk
  category sym
  meta     json
t = db.ins "tickets" {category::billing meta:{tries:3}}
(t.id == 1) | (fail "id 1 should be")
match t.category
  :billing -> log "ok sym"
  _ -> fail "sym :billing should be"
(t.meta.tries == 3) | (fail "json meta.tries 3 should be")
"#);
    });
}

#[test]
fn db_param_and_placeholder() {
    // q without a param + the $1 placeholder binds in SQLite without a rewrite + sym param.
    with_db_test("param_placeholder", || {
        run(r#"
use db
tbl items
  id   serial pk
  kind sym
db.ins "items" {kind::a}
db.ins "items" {kind::b}
all = db.q "select * from items"
(all.len == 2) | (fail "q without param 2 row")
only = db.q "select * from items where kind=$1" [:a]
(only.len == 1) | (fail "$1 sym param 1 row")
"#);
    });
}

// Declarative read builder (issue #78): db.from |> db.eq/cmp/order/limit
// |> db.all/first. A list value -> IN. Filter+range+order+paging without raw SQL.
#[test]
fn db_query_builder_reads() {
    with_db_test("query_builder", || {
        run(r#"
use db
tbl bookings
  id          serial pk
  tenant_id   int
  resource_id int
  status      sym
  start_at    str
db.ins "bookings" {tenant_id:1 resource_id:5 status::done start_at:"2026-06-01"}
db.ins "bookings" {tenant_id:1 resource_id:5 status::confirmed start_at:"2026-06-02"}
db.ins "bookings" {tenant_id:1 resource_id:7 status::pending start_at:"2026-06-03"}
db.ins "bookings" {tenant_id:2 resource_id:9 status::done start_at:"2026-06-04"}

# IN filter (list value) + order
in_rows = db.from "bookings" |> db.eq {tenant_id:1 status:[:pending :confirmed]} |> db.order :start_at |> db.all
(in_rows.len == 2) | (fail "IN-filter 2 row expected, ${in_rows.len}")
match in_rows.0.status
  :confirmed -> log "ok IN order"
  _ -> fail "order start_at wrong"

# cmp range + limit
rng = db.from "bookings" |> db.eq {tenant_id:1} |> db.cmp :start_at :ge "2026-06-02" |> db.limit 10 |> db.all
(rng.len == 2) | (fail "cmp >= 2 row expected, ${rng.len}")

# first — one or nil
one = db.from "bookings" |> db.eq {tenant_id:1 resource_id:7} |> db.first
(one != nil) | (fail "first returned nil")
match one.status
  :pending -> log "ok first"
  _ -> fail "first wrong row"

# first — no matching row → nil
none = db.from "bookings" |> db.eq {tenant_id:99} |> db.first
(none == nil) | (fail "first with no match expected nil")

# empty IN list → nothing
empty = db.from "bookings" |> db.eq {status:[]} |> db.all
(empty.len == 0) | (fail "empty IN 0 row expected")

# nil value → IS NULL ( = NULL never matches). a row with resource_id null.
db.ins "bookings" {tenant_id:1 resource_id:nil status::pending start_at:"2026-06-09"}
nulls = db.from "bookings" |> db.eq {tenant_id:1 resource_id:nil} |> db.all
(nulls.len == 1) | (fail "nil → IS NULL 1 row expected, ${nulls.len}")
"#);
    });
}

// Issue #104: when db.up was called with an empty condition map, build_update
// built a column-less "WHERE" (malformed SQL) and the whole table got updated.
// Like the guard in db.del, it now gives a clear error (instead of SQLite's raw
// "incomplete input").
#[test]
fn db_up_bosh_shart_rad_etiladi() {
    with_db_test("up_empty_where", || {
        let setup = "use db\ntbl t\n  id serial pk\n  n int\ndb.ins \"t\" {n:1}\n";
        let e = run_source(&format!("{setup}db.up \"t\" {{n:5}} {{}}\n")).unwrap_err();
        assert!(
            e.contains("db.up: condition map is empty"),
            "unexpected error: {e}"
        );
    });
}

// Issue #104: db.offset without LIMIT used to be silently ignored (SQLite requires
// LIMIT for OFFSET). Now it is applied correctly with LIMIT -1 OFFSET m.
#[test]
fn db_offset_limitsiz_qollanadi() {
    with_db_test("offset_no_limit", || {
        run(r#"
use db
tbl t
  id serial pk
  n  int
db.ins "t" {n:1}
db.ins "t" {n:2}
db.ins "t" {n:3}
# offset 1, no limit → skip the first, return the remaining 2.
rows = db.from "t" |> db.order :n |> db.offset 1 |> db.all
(rows.len == 2) | (fail "offset without LIMIT 2 row expected, ${rows.len}")
(rows.0.n == 2) | (fail "offset should skip the first needed, ${rows.0.n}")
"#);
    });
}

// Issue #104: a negative limit/offset gives unexpected behavior in SQLite (a
// negative LIMIT = unlimited). Now it is clearly rejected at the user level.
#[test]
fn db_manfiy_limit_offset_rad_etiladi() {
    with_db_test("neg_limit_offset", || {
        let setup = "use db\ntbl t\n  id serial pk\n  n int\n";
        let e1 = run_source(&format!(
            "{setup}db.from \"t\" |> db.limit (0 - 1) |> db.all\n"
        ))
        .unwrap_err();
        assert!(e1.contains("db.limit: negative"), "limit error: {e1}");
        let e2 = run_source(&format!(
            "{setup}db.from \"t\" |> db.offset (0 - 3) |> db.all\n"
        ))
        .unwrap_err();
        assert!(e2.contains("db.offset: negative"), "offset error: {e2}");
    });
}

// Aggregation builder: group + count/sum + conditional agg (count_if/sum_if).
#[test]
fn db_query_builder_agg() {
    with_db_test("query_builder_agg", || {
        run(r#"
use db
tbl bookings
  id          serial pk
  tenant_id   int
  resource_id int
  status      sym
  total_cents money
db.ins "bookings" {tenant_id:1 resource_id:5 status::done total_cents:5000}
db.ins "bookings" {tenant_id:1 resource_id:5 status::confirmed total_cents:3000}
db.ins "bookings" {tenant_id:1 resource_id:7 status::pending total_cents:1000}

# group + count + sum, order desc
ag = db.from "bookings" |> db.eq {tenant_id:1 status:[:done :confirmed]} |> db.group :resource_id |> db.count :n |> db.sum :total_cents :rev |> db.order :rev :desc |> db.agg
(ag.len == 1) | (fail "agg 1 guruh expected, ${ag.len}")
(ag.0.resource_id == 5) | (fail "agg resource_id 5")
(ag.0.n == 2) | (fail "agg count 2, ${ag.0.n}")
(ag.0.rev == 8000) | (fail "agg sum 8000, ${ag.0.rev}")

# conditional agg (overview, no group) → a single row
ov = db.from "bookings" |> db.eq {tenant_id:1} |> db.count_if {status::confirmed} :confirmed |> db.count_if {status::pending} :pending |> db.sum_if :total_cents {status::done} :revenue |> db.agg_row
(ov.confirmed == 1) | (fail "count_if confirmed 1, ${ov.confirmed}")
(ov.pending == 1) | (fail "count_if pending 1, ${ov.pending}")
(ov.revenue == 5000) | (fail "sum_if revenue 5000, ${ov.revenue}")

# empty tenant: count_if must return 0 (not nil — COUNT semantics)
empty_ov = db.from "bookings" |> db.eq {tenant_id:99} |> db.count_if {status::done} :done |> db.agg_row
(empty_ov.done == 0) | (fail "empty count_if 0 expected (nil not), ${empty_ov.done}")
"#);
    });
}

// str.sym: string -> symbol (turning query-string statuses into a sym filter).
#[test]
fn str_sym_conversion() {
    run(r#"
(str.sym "done" == :done) | (fail "str.sym done")
syms = (str.split "pending,confirmed" ",").map \s -> str.sym s
(syms.0 == :pending) | (fail "str.sym split 0")
(syms.1 == :confirmed) | (fail "str.sym split 1")
(str.sym " done " == :done) | (fail "str.sym trim")
"#);
}

// --- Issue #82: tbl declarative schema migration + index/uniq ---
