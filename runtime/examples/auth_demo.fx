# auth battery — JWT signing/verification + password hash (issue #69).
#
# The signing key $AUTH_SECRET is found AUTOMATICALLY from env (matching the
# db -> $DATABASE_URL, ai -> ANTHROPIC_API_KEY pattern). Nothing to configure:
#
#   export AUTH_SECRET=secret-key
#   fluxon run examples/auth_demo.fx
#
# (The server does NOT BLOCK — this demo exits directly. For a real
#  auth flow see examples/middleware.fx.)

use auth

# --- Password hash (registration / login) ---
# Salt is automatic, constant-time comparison — the agent cannot get it wrong.
hash = auth.hash "user-password-123"
log "hash: ${hash}"

ok = auth.check "user-password-123" hash
log "correct password -> ${ok}"               # true

wrong = auth.check "other-password" hash
log "incorrect password -> ${wrong}"          # false

# --- JWT signing (after a successful login) ---
# iat (issued at) + exp (expiry, default 24 hours) are auto-added to the payload map.
token = auth.jwt {sub: "u_42" tenant: "acme" role: "admin"}
log "token: ${token}"

# If you need a short expiry (1 hour) — second {exp:seconds} opt:
short = auth.jwt {sub: "u_42"} {exp: 3600}
log "1-hour token length: ${(str.split short ".").len} segments"

# --- JWT verification (on every protected request) ---
# Signature + expiry (exp) are checked AUTOMATICALLY. If invalid err -> 401.
claims = auth.verify token
log "claims.sub = ${claims.sub}"
log "claims.tenant = ${claims.tenant}"
log "claims.role = ${claims.role}"

log "auth demo finished"
