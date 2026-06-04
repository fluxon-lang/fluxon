# moderation.flux — AI avtomatik moderatsiya
# Har bir xabar yuborilganda toxicity/spam tekshiruvi o'tkaziladi.

use ai as ai_mod
use db

# Moderatsiya natijasi tipi:
#   { label: :ok | :toxic | :spam, confidence: flt, reason: str }

exp fn moderate_message body channel_id user_id
  # ai.json orqali klassifikatsiya
  result = ai_mod.json "Quyidagi chat xabarini moderatsiya qil. Natija faqat JSON:
  Xabar: ${body}

  Agar xabar zaharli (haqorat, nafrat, tahdid) bo'lsa — label: toxic
  Agar spam/reklama bo'lsa — label: spam
  Aks holda — label: ok
  " {
    label:      ":ok|:toxic|:spam"
    confidence: flt
    reason:     str
  }

  label      = result.label
  confidence = result.confidence ?? 0.0
  reason     = result.reason ?? ""

  # Yuqori ishonch bilan toxik — bloklash
  if label == :toxic & confidence > 0.85
    db.ins "moderation_logs" {
      channel:    channel_id
      user:       user_id
      body:       body
      label:      "toxic"
      confidence: confidence
      reason:     reason
      action:     "blocked"
      created:    time.now
    }
    ret {allowed:false action::blocked reason:reason confidence:confidence}

  # O'rta ishonch bilan toxik — flaglash
  if label == :toxic & confidence >= 0.6
    ret {allowed:true action::flagged reason:reason confidence:confidence}

  # Spam yuqori ishonch bilan — bloklash
  if label == :spam & confidence > 0.80
    ret {allowed:false action::blocked reason:"spam aniqlandi" confidence:confidence}

  # Spam o'rta ishonch bilan — flaglash
  if label == :spam & confidence >= 0.5
    ret {allowed:true action::flagged reason:"ehtimoliy spam" confidence:confidence}

  # Hammasi yaxshi
  ret {allowed:true action::ok reason:"" confidence:confidence}

# Moderatsiya jadvali (tbl ta'rifi shu yerda — schema.flux'dan alohida, chunki
# bu modul ichki logga bog'liq)
tbl moderation_logs
  id         serial pk
  channel    int ref:channels.id
  user       int ref:users.id
  body       str
  label      str
  confidence flt
  reason     str
  action     str
  created    now
