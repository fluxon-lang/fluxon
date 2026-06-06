# AI chat — terminalda ishlaydigan kichik suhbat (REPL).
#
# Kalit AVTOMATIK topiladi (hech narsa sozlash shart emas):
#   .env yoki muhitda ANTHROPIC_API_KEY bo'lsa -> Claude
#   OPENAI_API_KEY bo'lsa            -> GPT
# Model: $AI_MODEL ?? provayder default. Override: $AI_PROVIDER (anthropic|openai).
#
# Ishga tushirish — `.env` qaysi katalogda bo'lsa, flux O'SHA katalogdan
# ishga tushganda topadi (env_lookup joriy katalogdagi `.env`ni o'qiydi).
#   # .env loyiha root'ida bo'lsa:
#   cd <loyiha-root> && runtime/target/release/flux run runtime/examples/ai_chat.fx
#   # yoki kalitni muhitga eksport qilib, istalgan katalogdan:
#   export ANTHROPIC_API_KEY=sk-ant-...   # yoki OPENAI_API_KEY=sk-...
#   flux run examples/ai_chat.fx
#
# Chiqish: "chiq", "exit" yoki "/q" deb yozing (yoki Ctrl+D).

use ai io

io.print "Flux AI chat — savolingizni yozing ('chiq' = tugatish)\n\n"

# Suhbat tarixi — har yangi xabar va javob shu yerga qo'shiladi, shunda model
# kontekstni eslab qoladi (ai.run msgs orqali ko'p qadamli suhbat).
tarix <- []

each i in 1..1000
  savol = io.prompt "siz> "

  # EOF (Ctrl+D) -> nil; yoki chiqish so'zlari -> tugatamiz.
  if savol == nil
    io.print "\nxayr!\n"
    ret nil
  if savol == "chiq" | savol == "exit" | savol == "/q"
    io.print "xayr!\n"
    ret nil
  # bo'sh qatorni o'tkazib yuboramiz.
  if savol == ""
    skip

  # Foydalanuvchi xabarini tarixga qo'shamiz.
  tarix <- tarix.push {role::user content:savol}

  # Modeldan javob — ai.run bitta qadam qaytaradi. Bu chatda tool yo'q,
  # shuning uchun har doim :final keladi.
  r = ai.run tarix []

  javob = r.text
  io.print "ai > ${javob}\n\n"

  # Model javobini ham tarixga qo'shamiz (keyingi savolda kontekst bo'lsin).
  tarix <- tarix.push {role::assistant content:javob}
