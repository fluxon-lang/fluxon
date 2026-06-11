# 14 — crypto battery (issue #131).
# Ishga: ./target/release/fluxon run tests-fx/14_crypto.fx
#
# crypto.sha256 s     -> SHA-256 hex (kichik harf)
# crypto.hmac key msg -> HMAC-SHA256 hex (webhook imzo tekshirish)
# crypto.b64 s        -> base64 kodlash
# crypto.b64d s       -> base64 ochish (padding ixtiyoriy, url-safe ham)
# crypto.hex s        -> baytlarning hex ko'rinishi
# crypto.uuid         -> UUID v4 (kriptografik manba)

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

# --- sha256 (FIPS 180-2 ma'lum vektor) ---
eq (crypto.sha256 "abc") "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad" "crypto.sha256 ma'lum vektor"

# --- hmac (RFC 4231 test case 2) — webhook imzo stsenariysi ---
sig = crypto.hmac "Jefe" "what do ya want for nothing?"
eq sig "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843" "crypto.hmac RFC 4231 vektor"

# Imzo tekshirish naqshi: kelgan imzo bilan qayta hisoblangani teng bo'lsin.
kelgan = crypto.hmac "webhook-siri" "to'lov tasdiqlandi"
truthy ((crypto.hmac "webhook-siri" "to'lov tasdiqlandi") == kelgan) "hmac deterministik (imzo tekshirish)"
truthy ((crypto.hmac "boshqa-kalit" "to'lov tasdiqlandi") != kelgan) "boshqa kalit -> boshqa imzo"

# --- base64 ---
eq (crypto.b64 "salom dunyo") "c2Fsb20gZHVueW8=" "crypto.b64 kodlash"
eq (crypto.b64d "c2Fsb20gZHVueW8=") "salom dunyo" "crypto.b64d padding bilan"
eq (crypto.b64d "c2Fsb20gZHVueW8") "salom dunyo" "crypto.b64d padding'siz ham"
eq (crypto.b64d (crypto.b64 "aylanma ✓")) "aylanma ✓" "b64 roundtrip (unicode)"

# --- hex ---
eq (crypto.hex "abz") "61627a" "crypto.hex"

# --- uuid ---
u = crypto.uuid
eq (str.len u) 36 "crypto.uuid uzunligi 36"
parts = str.split u "-"
eq parts.len 5 "crypto.uuid 5 segment"
truthy (u != crypto.uuid) "crypto.uuid har gal boshqa"

# --- Yakun ---
if fails == 0
  log "=== 14_crypto: HAMMASI O'TDI ==="
else
  log "=== 14_crypto: ${fails} TEST YIQILDI ==="
