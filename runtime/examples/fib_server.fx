# fib_server.fx — CPU-heavy recursive handler (Arc contention benchmark).
# GET /fib/:n -> computes fib(n). The heavy recursion clones the global
# `fib` Arc<FnValue> on every call -> 8 threads contend on the atomic refcount.
use http

fn fib n
  if n < 2
    ret n
  (fib (n - 1)) + (fib (n - 2))

http.on :get "/fib/:n" \req ->
  n = str.int req.params.n
  rep 200 {n:n result:(fib n)}

http.serve 8099
