# 13 - auth battery (JWT + password hash, issue #69).
# Run: AUTH_SECRET=secret-key ./target/release/fluxon run tests-fx/13_auth.fx
#
# auth.jwt {payload} [{exp:N}] -> signed JWT (HS256).
# auth.verify token            -> payload map (signature + exp auto-checked), or err.
# auth.hash "password"         -> argon2id PHC string (salt embedded).
# auth.check "password" hash   -> bool (constant-time comparison).
#
# The signing key is auto-found from $AUTH_SECRET env (matches the db/ai pattern).

use auth

fails <- 0

fn eq got want label
  if got == want
    log "ok  ${label} = ${got}"
  else
    log "FAIL ${label}: got=${got} want=${want}"
    fails <- fails + 1

fn truthy v label
  if v
    log "ok  ${label}"
  else
    log "FAIL ${label}: got=${v}"
    fails <- fails + 1

# --- Password hash + check ---
hash = auth.hash "user-password-123"
truthy (str.has hash "argon2id") "auth.hash argon2id PHC string"
truthy (auth.check "user-password-123" hash) "auth.check correct password -> true"
eq (auth.check "other" hash) false "auth.check wrong password -> false"
eq (auth.check "x" "broken-hash") false "auth.check broken hash -> false (not an error)"

# Same password yields a different hash each time (random salt).
h2 = auth.hash "user-password-123"
truthy (hash != h2) "auth.hash random salt (different each time)"

# --- JWT sign + verify (roundtrip) ---
token = auth.jwt {sub: "u1" tenant: "t1" role: "admin"}
parts = str.split token "."
eq parts.len 3 "JWT 3 segments (header.payload.signature)"

claims = auth.verify token
eq claims.sub "u1" "verify -> sub preserved"
eq claims.tenant "t1" "verify -> tenant preserved"
eq claims.role "admin" "verify -> role preserved"
truthy (claims.exp > claims.iat) "iat/exp auto-added (exp > iat)"

# --- {exp:N} opt ---
short = auth.jwt {sub: "u2"} {exp: 3600}
sc = auth.verify short
truthy (sc.exp == (sc.iat + 3600)) "{exp:3600} -> exp = iat + 3600"

# --- End ---
if fails == 0
  log "=== 13_auth: ALL PASSED ==="
else
  log "=== 13_auth: ${fails} TESTS FAILED ==="
