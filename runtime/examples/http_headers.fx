# Custom response headers (issue #16) — rep's optional 3rd argument map.
#
# In the header name write `_` instead of a hyphen (a Fluxon map key cannot contain
# a hyphen); the runtime converts it to `-`: content_type -> Content-Type. The name
# is case-insensitive. This is symmetric with reading — req.headers also maps `-` -> `_`.
#
# The server blocks; running it: cargo run -- run examples/http_headers.fx
# then in another terminal: curl -i http://127.0.0.1:8080/html
use http

# 1) Custom Content-Type — return HTML (a str body defaults to text/plain).
http.on :get "/html" \req ->
  rep 200 "<h1>Hello Fluxon</h1>" {content_type:"text/html"}

# 2) Redirect — Location header (302). URL-shortener pattern.
http.on :get "/go" \req ->
  rep 302 nil {location:"https://example.com"}

# 3) Set a cookie/session. Multiple cookies — a list value (each one a
#    separate Set-Cookie line; RFC 7230 forbids a comma-separated list).
http.on :get "/login" \req ->
  rep 200 {ok:true} {set_cookie:["session=abc123" "theme=dark"]}

# 4) CSV export — custom Content-Type + download filename.
http.on :get "/export" \req ->
  rep 200 "id,name\n1,Ali\n2,Vali" {
    content_type:"text/csv"
    content_disposition:"attachment; filename=\"data.csv\""
  }

log "http://127.0.0.1:8080 — /html /go /login /export"
http.serve 8080
