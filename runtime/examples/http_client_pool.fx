# Reusable HTTP klient pool misoli.
#
# 1-terminalda lokal serverni ishga tushiring:
#   cargo run -- run examples/server.fx
#
# Ketma-ket sinov:
#   cargo run -- run examples/http_client_pool.fx
#
# Parallel sinov (shu sodda Flux klient API bir nechta parallel chaqiruvda ham
# o'zgarmaydi; har jarayon ichida http.get global Hyper client poolini ishlatadi):
#   for i in 1 2 3 4; do cargo run --quiet -- run examples/http_client_pool.fx & done; wait

use http

url = "http://127.0.0.1:8080/health"

fn get_health label
  resp = http.get url
  log "${label}: status=${resp.status} ok=${resp.body.ok}"
  resp

log "ketma-ket http.get chaqiruvlari"
each i in 1..3
  get_health "seq-${i}"

log "parallel sinov uchun yuqoridagi for ... & ... wait komandasi bilan shu faylni bir nechta nusxada ishga tushiring"
