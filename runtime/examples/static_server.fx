# Static file serving demonstration (issue #134) — frontend build + API together.
# Running:  fluxon run examples/static_server.fx
# (the side folder examples/public/ ships with sample files)
# Testing (in another terminal):
#   curl -s -i localhost:8080/assets/app.css     # file + Content-Type automatic
#   curl -s -i localhost:8080/                   # SPA: dist/index.html
#   curl -s -i localhost:8080/api/health         # explicit route beats static

use http

# Prefix -> folder: /assets/app.css -> ./public/app.css
# The directory is resolved relative to the folder the script file is in.
http.static "/assets" "./public"

# SPA mode: an unmatched route falls back to ./dist/index.html
# (the frontend router handles it). Explicit routes still win.
# http.static "/" "./dist" {spa: true}

http.on :get "/api/health" \req ->
  rep 200 {ok: true}

log "Static server starting on port 8080..."
http.serve 8080
