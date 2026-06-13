# 14 - crypto battery (issue #131).
# Run: ./target/release/fluxon run tests-fx/14_crypto.fx
#
# crypto.sha256 s     -> SHA-256 hex (lowercase)
# crypto.hmac key msg -> HMAC-SHA256 hex (webhook signature verification)
# crypto.b64 s        -> base64 encode
# crypto.b64d s       -> base64 decode (padding optional, url-safe too)
# crypto.hex s        -> hex representation of the bytes
# crypto.uuid         -> UUID v4 (cryptographic source)

use crypto

fails <- 0

fn eq got want label
  if got == want
    log "ok  ${label}"
  else
    log "FAIL ${label}: got=${got} want=${want}"
    fails <- fails + 1

fn truthy v label
  if v
    log "ok  ${label}"
  else
    log "FAIL ${label}: got=${v}"
    fails <- fails + 1

# --- sha256 (FIPS 180-2 known vector) ---
eq (crypto.sha256 "abc") "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad" "crypto.sha256 known vector"

# --- hmac (RFC 4231 test case 2) - webhook signature scenario ---
sig = crypto.hmac "Jefe" "what do ya want for nothing?"
eq sig "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843" "crypto.hmac RFC 4231 vector"

# Signature-verification pattern: the incoming signature must equal the recomputed one.
incoming = crypto.hmac "webhook-secret" "payment confirmed"
truthy ((crypto.hmac "webhook-secret" "payment confirmed") == incoming) "hmac deterministic (signature check)"
truthy ((crypto.hmac "other-key" "payment confirmed") != incoming) "different key -> different signature"

# --- base64 ---
eq (crypto.b64 "salom dunyo") "c2Fsb20gZHVueW8=" "crypto.b64 encode"
eq (crypto.b64d "c2Fsb20gZHVueW8=") "salom dunyo" "crypto.b64d with padding"
eq (crypto.b64d "c2Fsb20gZHVueW8") "salom dunyo" "crypto.b64d without padding too"
eq (crypto.b64d (crypto.b64 "roundtrip ✓")) "roundtrip ✓" "b64 roundtrip (unicode)"

# --- hex ---
eq (crypto.hex "abz") "61627a" "crypto.hex"

# --- uuid ---
u = crypto.uuid
eq (str.len u) 36 "crypto.uuid length 36"
parts = str.split u "-"
eq parts.len 5 "crypto.uuid 5 segments"
truthy (u != crypto.uuid) "crypto.uuid different each time"

# --- End ---
if fails == 0
  log "=== 14_crypto: ALL PASSED ==="
else
  log "=== 14_crypto: ${fails} TESTS FAILED ==="
