# Reusable HTTP client pool example.
#
# In terminal 1, start the local server:
#   cargo run -- run examples/server.fx
#
# Sequential test:
#   cargo run -- run examples/http_client_pool.fx
#
# Parallel test (this simple Fluxon client API does not change even under several
# parallel calls; within each process http.get uses the global Hyper client pool):
#   for i in 1 2 3 4; do cargo run --quiet -- run examples/http_client_pool.fx & done; wait

use http

url = "http://127.0.0.1:8080/health"

fn get_health label
  resp = http.get url
  log "${label}: status=${resp.status} ok=${resp.body.ok}"
  resp

log "sequential http.get calls"
each i in 1..3
  get_health "seq-${i}"

log "for a parallel test, run this file in several copies with the for ... & ... wait command above"
