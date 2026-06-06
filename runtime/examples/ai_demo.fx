# AI demo — LLM birlamchi primitivlari (Anthropic Messages API).
#
# Ishga tushirishdan oldin $AI_KEY ni belgilang (OS env yoki .env faylda):
#   export AI_KEY=sk-ant-...
#   flux run examples/ai_demo.fx
#
# Model: $AI_MODEL ?? "claude-opus-4-8". Boshqa model uchun:
#   export AI_MODEL=claude-sonnet-4-6
#
# DIQQAT: bu misol haqiqiy API chaqiradi (token sarflaydi). Kalit yo'q bo'lsa
# aniq xato beradi, tarmoqqa chiqmaydi.

use ai reg

# 1) ai.ask — oddiy savol, matn javob.
javob = ai.ask "Bir jumlada: Flux tili nima uchun yaxshi?"
log "ask: ${javob}"

# 2) ai.json — strukturalangan chiqish. Schema map beriladi, model unga MOS
#    JSON qaytaradi. Natijada `_` metadata (conf/tokens/cost/ms) ham bo'ladi.
r = ai.json "Ushbu buyurtmani ajrat: 3 dona olma, 2 dona non" {
  mahsulotlar: [{nom:str soni:int}]
}
log "json natija: ${r.mahsulotlar}"
log "ishonch: ${r._.conf}  tokenlar: ${r._.tokens}  narx: ${r._.cost}  vaqt(ms): ${r._.ms}"

# Ishonch bo'yicha qaror (spec naqshi):
if r._.conf > 0.85
  log "yuqori ishonch -> avtomatik qabul"
elif r._.conf >= 0.6
  log "o'rta ishonch -> tasdiqlash so'rash"
else
  log "past ishonch -> odamga uzatish"

# 3) ai.run — tool-loop'ning BIR qadami. Tool'ni model O'ZI bajarmaydi:
#    loop sizniki (log/narx/tasdiq nazorati). Tool'ni reg.call orqali Flux
#    tomonda chaqirasiz, natijani msgs'ga qo'shasiz.

# Tool funksiyasini registrga qo'shamiz (reg dinamik dispatch).
reg.add "ob_havo" \args ->
  # Haqiqiy holatda bu http.get qilardi; demo uchun qat'iy javob.
  "${args.shahar}da 25 daraja, quyoshli"

# Tool ta'rifi: nom, tavsif, parametrlar (sodda {nom:tip} -> JSON-schema).
tools = [{
  name: "ob_havo"
  desc: "Berilgan shahardagi joriy ob-havo"
  params: {shahar:str}
}]

# Suhbat tarixi — birinchi xabar.
msgs <- [{role::user content:"Toshkentda ob-havo qanday?"}]

# Tool-loop: model :final qaytarmaguncha (yoki limit) aylanamiz.
each i in 1..10
  r = ai.run msgs tools
  if r.kind == :final
    log "yakuniy javob: ${r.text}"
    ret r.text
  # r.kind == :call -> model tool chaqirmoqchi, uni Flux tomonda bajaramiz.
  log "tool chaqiruvi: ${r.tool} args=${r.args}"
  natija = reg.call r.tool r.args
  # Model javobini (tool_use) va tool natijasini tarixga qo'shamiz.
  msgs <- msgs.push {role::assistant content:[{type:"tool_use" id:r.id name:r.tool input:r.args}]}
  msgs <- msgs.push {role::tool id:r.id content:natija}
