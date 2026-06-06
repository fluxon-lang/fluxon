# 10 — ai battery (LLM primitiv). TARMOQSIZ test: haqiqiy API chaqirmaymiz
# (token sarflamaslik + CI'da kalit yo'q). Faqat shu narsalarni sinaymiz:
#   - `ai` modul nomi o'zgaruvchi bilan SHADOW qilinsa, oddiy map sifatida o'qiladi
#   - tool-loop'ning FLUX TOMONI (reg.call bilan tool bajarish) ishlaydi
#
# ai.ask/ai.json/ai.run ning haqiqiy chaqiruvi $AI_KEY talab qiladi va tarmoqqa
# chiqadi — uni examples/ai_demo.fx qo'lda sinaydi (kalit bilan).

use reg

fails <- 0
fn ok label -> log "ok  ${label}"
fn bad label
  log "FAIL ${label}"
  fails <- fails + 1

# --- shadowing: `ai` o'zgaruvchi bo'lsa, modul emas ---
# ai.ask "..." (argumentli) eval_call'da lookup tekshiradi: o'zgaruvchi bo'lsa
# dispatch'ga ketmaydi. Argumentsiz ai.ask esa Field — to'g'ridan map'dan o'qiladi.
ai = {ask:"shadowed" model:"yo'q"}
if ai.ask == "shadowed"
  ok "ai shadow: ai.ask map maydonidan o'qildi"
else
  bad "ai shadow buzildi got=${ai.ask}"

if ai.model == "yo'q"
  ok "ai shadow: ai.model = ${ai.model}"
else
  bad "ai.model got=${ai.model}"

# --- tool-loop FLUX tomoni: ai.run :call qadamini simulyatsiya ---
# ai.run model bilan {kind::call tool args id} qaytaradi. Loop tool'ni reg.call
# bilan bajarib, natijani msgs'ga qo'shadi. Bu yerda model javobini QO'LDA yasab,
# reg.call + msgs.push mantig'ini sinaymiz (tarmoqsiz).

reg.add "ob_havo" \args -> "${args.shahar}da 25 daraja"

# Model "tool chaqirdi" deb faraz qilamiz (ai.run shunday map qaytaradi):
qadam = {kind::call tool:"ob_havo" args:{shahar:"Toshkent"} id:"toolu_1"}

# natija'ni tashqarida e'lon qilamiz — `=` if blokida shaffof (tashqini yangilaydi),
# lekin oldindan mavjud bo'lsa pastda ham ko'rinadi.
natija <- ""
if qadam.kind == :call
  natija <- reg.call qadam.tool qadam.args
  if natija == "Toshkentda 25 daraja"
    ok "tool-loop: reg.call qadam natijasi = ${natija}"
  else
    bad "tool-loop natija got=${natija}"
else
  bad "qadam.kind != :call"

# msgs'ga tool natijasini qo'shish (suhbat tarixini o'stirish)
msgs <- [{role::user content:"ob-havo?"}]
msgs <- msgs.push {role::tool id:qadam.id content:natija}
if msgs.len == 2 & msgs.1.role == :tool
  ok "tool-loop: msgs tarixi o'sdi (${msgs.len} xabar)"
else
  bad "msgs tarix got len=${msgs.len}"

# --- final qadam shakli ---
final = {kind::final text:"javob tayyor"}
if final.kind == :final & final.text == "javob tayyor"
  ok "ai.run :final shakli to'g'ri"
else
  bad ":final shakli buzildi"

# --- Yakun ---
if fails == 0
  log "=== 10_ai: HAMMASI O'TDI ==="
else
  log "=== 10_ai: ${fails} TEST YIQILDI ==="
