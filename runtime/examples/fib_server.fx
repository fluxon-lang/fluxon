# fib_server.fx — CPU-og'ir rekursiv handler (Arc contention benchmark).
# GET /fib/:n -> fib(n) hisoblaydi. Og'ir rekursiya har chaqiruvda global
# `fib` Arc<FnValue> ni klonlaydi -> 8 thread atomik refcount'da urishadi.
use http

fn fib n
  if n < 2
    ret n
  (fib (n - 1)) + (fib (n - 2))

http.on :get "/fib/:n" \req ->
  n = str.int req.params.n
  rep 200 {n:n result:(fib n)}

http.serve 8099
