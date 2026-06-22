# 02 - List/Map methods + modules (str, math, rand, json).

fails <- 0
fn eq got want label
  if got == want
    log "ok  ${label} = ${got}"
  else
    log "FAIL ${label}: got=${got} want=${want}"
    fails <- fails + 1

# --- List methods ---
l = [3 1 4 1 5 9 2 6]
eq l.len 8 "list.len"
eq l.0 3 "list index .0"
eq l[2] 4 "list index [2]"
eq (l.has 9) true "list.has true"
eq (l.has 7) false "list.has false"

# index - position (-1 if not found), pairs with has bool
eq (l.index 4) 2 "list.index found"
eq ((l.index 7) == -1) true "list.index not found -> -1"

# find - first element matching the predicate (nil if none)
eq (l.find \x -> x > 4) 5 "list.find found"
eq (l.find \x -> x > 99) nil "list.find not found -> nil"

evens = l.filter \x -> x % 2 == 0
eq evens [4 2 6] "list.filter"

doubled = [1 2 3].map \x -> x * 2
eq doubled [2 4 6] "list.map"

total = [1 2 3 4 5].reduce 0 \acc x -> acc + x
eq total 15 "list.reduce"

eq ([10 20 30 40].slice 1 3) [20 30] "list.slice"
eq (["a" "b" "c"].join "-") "a-b-c" "list.join"

# push - returns a new list (canonical: l.push x)
base = [1 2]
grown = base.push 3
eq grown [1 2 3] "list.push"

# chain: filter -> map -> reduce (canonical - with intermediate bindings)
src = [1 2 3 4 5 6]
ev = src.filter \x -> x % 2 == 0
sq = ev.map \x -> x * x
sq_sum = sq.reduce 0 \a x -> a + x
eq sq_sum 56 "chain filter->map->reduce (4+16+36)"

# --- Map methods ---
m = {a:1 b:2 c:3}
eq m.len 3 "map.len"
eq m.a 1 "map .key"
eq m["b"] 2 "map [key]"
eq (m.has "c") true "map.has true"
eq (m.has "z") false "map.has false"
eq m.keys ["a" "b" "c"] "map.keys"
eq m.vals [1 2 3] "map.vals"

m2 = m.set "d" 4
eq m2.d 4 "map.set adds"
eq (m.has "d") false "map.set immutable (original unchanged)"

m3 = m.del "a"
eq (m3.has "a") false "map.del removes"

# spread + dynamic key
merged = {...m x:9}
eq merged.x 9 "map spread + new"
eq merged.a 1 "map spread keeps"

k = "dyn"
dynm = {[k]:100}
eq dynm.dyn 100 "map dynamic key"

# --- index assignment (issue #220): m[k] = v writes IN PLACE ---
cnt = {}
cnt["a"] = 1
cnt["a"] = (cnt["a"] ?? 0) + 9
eq cnt.a 10 "map index assign in place"
cnt.b = 2
eq cnt.b 2 "map field assign m.key = v"

# the canonical word-count accumulator
words = ["red" "blue" "red"]
wc = {}
each w in words
  wc[w] = (wc[w] ?? 0) + 1
eq wc.red 2 "map accumulator each"
eq wc.blue 1 "map accumulator each (single)"

# list element write (in-range)
ml = [1 2 3]
ml[1] = 20
eq ml.1 20 "list index assign in place"
eq ml.len 3 "list index assign keeps length"

# --- str module ---
eq (str.len "hello") 5 "str.len"
eq (str.up "abc") "ABC" "str.up"
eq (str.low "XYZ") "xyz" "str.low"
eq (str.slice "abcdef" 1 4) "bcd" "str.slice"
eq (str.split "a,b,c" ",") ["a" "b" "c"] "str.split"
eq (str.has "hello world" "world") true "str.has true"
eq (str.has "hello" "xyz") false "str.has false"
eq (str.int "42") 42 "str.int"
eq (str.str 42) "42" "str.str"

# --- math module ---
eq (math.floor 3.7) 3 "math.floor"
eq (math.ceil 3.2) 4 "math.ceil"
eq (math.abs (-5)) 5 "math.abs"
eq (math.round 3.5) 4 "math.round"

# --- rand module (range check) ---
r = rand.int 1 10
if r >= 1 & r <= 10
  log "ok  rand.int in [1,10] = ${r}"
else
  log "FAIL rand.int out of range: ${r}"
  fails <- fails + 1

rs = rand.str 8
if (str.len rs) == 8
  log "ok  rand.str len 8 = ${rs}"
else
  log "FAIL rand.str wrong len: ${rs}"
  fails <- fails + 1

# --- json module: roundtrip ---
obj = {name:"Ali" age:30 tags:["a" "b"] active:true}
enc = json.enc obj
dec = json.dec enc
eq dec.name "Ali" "json roundtrip str"
eq dec.age 30 "json roundtrip int"
eq dec.tags ["a" "b"] "json roundtrip list"
eq dec.active true "json roundtrip bool"

# --- End ---
if fails == 0
  log "=== 02_collections_modules: ALL PASSED ==="
else
  log "=== 02_collections_modules: ${fails} TESTS FAILED ==="
