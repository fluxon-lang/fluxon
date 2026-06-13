# 05 - http client: sends requests to 05_http_server.fx and checks the responses.
# The server must be running on port 8123.
use http

fails <- 0
fn eq got want label
  if got == want
    log "ok  ${label} = ${got}"
  else
    log "FAIL ${label}: got=${got} want=${want}"
    fails <- fails + 1

base = "http://127.0.0.1:8123"

# GET /health -> {ok:true}
r = http.get "${base}/health"
eq r.status 200 "GET /health status"
eq r.body.ok true "GET /health body.ok"

# GET /echo/:id -> param
r2 = http.get "${base}/echo/42"
eq r2.status 200 "GET /echo/:id status"
eq r2.body.id "42" "GET /echo param"

# GET /q?name=Ali -> query
r3 = http.get "${base}/q?name=Ali"
eq r3.body.name "Ali" "GET query param"

# no query -> default
r4 = http.get "${base}/q"
eq r4.body.name "none" "GET query missing -> default"

# POST /sum {a,b} -> JSON body is read
r5 = http.post "${base}/sum" {a:7 b:8}
eq r5.status 201 "POST /sum status 201"
eq r5.body.sum 15 "POST /sum body parsed (7+8)"

# POST /strict empty -> fail 400
r6 = http.post "${base}/strict" {}
eq r6.status 400 "POST /strict missing email -> 400"
eq r6.body.error "email required" "fail message in body.error"

# POST /strict with email -> 201
r7 = http.post "${base}/strict" {email:"a@b.uz"}
eq r7.status 201 "POST /strict valid -> 201"
eq r7.body.email "a@b.uz" "POST /strict echoes email"

# GET /text -> text response (not JSON)
r8 = http.get "${base}/text"
eq r8.status 200 "GET /text status"
eq r8.body "hello world" "GET /text plain body"

# res.headers - response headers read as a map (lowercase keys)
# for a hyphenated key (content-type) we use m[k] indexing (not m.k).
r9 = http.get "${base}/health"
ct = r9.headers["content-type"]
eq (str.has ct "application/json") true "res.headers content-type"

# follow:false (default) - raw 302 returned, Location header read
r10 = http.get "${base}/r1"
eq r10.status 302 "default follow off -> raw 302"
eq r10.headers.location "/r2" "res.headers.location read"

# follow:true - redirect chain is followed (/r1 -> /r2 -> /dest)
r11 = http.get "${base}/r1" {follow:true}
eq r11.status 200 "follow:true -> final 200"
eq r11.body.arrived true "follow:true -> /dest body"
eq r11.hops 2 "follow:true -> 2 hops counted"

# custom request headers - the server echoes them (issue #34)
r12 = http.get "${base}/echo-headers" {
  headers: {
    "x-api-key": "secret-key"
    "anthropic-version": "2023-06-01"
  }
}
eq r12.status 200 "custom headers -> 200"
eq r12.body.key "secret-key" "x-api-key reached the server"
eq r12.body.ver "2023-06-01" "anthropic-version reached the server"

# no header given - the server returns a default (no regression)
r13 = http.get "${base}/echo-headers"
eq r13.body.key "none" "request without headers -> server default"

# custom response headers (issue #16): rep 3rd-argument map.
# content_type -> overrides the body default Content-Type (text/plain).
r14 = http.get "${base}/html"
eq r14.status 200 "GET /html status"
eq r14.headers["content-type"] "text/html" "rep custom content-type"
eq r14.headers["x-powered-by"] "fluxon" "rep custom x-powered-by (_->-)"
eq r14.body "<h1>Hello</h1>" "rep body preserved with custom header"

# repeated Set-Cookie: the server sends two separate lines, the client now
# returns them as a List (issue #101) - symmetric with the List on the write side.
r15 = http.get "${base}/cookies"
sc = r15.headers["set-cookie"]
eq sc.len 2 "repeated set-cookie both lines arrived"
eq sc.0 "a=1" "set-cookie[0]"
eq sc.1 "b=2" "set-cookie[1]"

if fails == 0
  log "=== 05_http: ALL PASSED ==="
else
  log "=== 05_http: ${fails} TESTS FAILED ==="
