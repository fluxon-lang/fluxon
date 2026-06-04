# Incoming WhatsApp message → AI classify+extract+route → reply.
use http db ai json
use ./tools

# Extraction schema for orders (ai.json fills this typed shape).
ext = {
  intent: ":new_order|:question|:complaint|:greeting|:other"
  items:  [{product:str qty:int}]
  delivery_date: str
}

exp fn handle req
  m = req.body                         # {from:ph name:str text:str}
  c = ensure_customer m.from m.name
  msg = db.ins "messages" {cust:c.id dir::in body:m.text}

  # One AI call: classify + extract + confidence (conf comes back in _.conf).
  cat = tools.get_product_catalog c.owner
  p = "Mijoz xabari (o'zbekcha): \"${m.text}\".
Mahsulotlar: ${json.enc cat}.
Niyatni aniqla va buyurtmani ajrat. Narx YOZMA — narx bazadan olinadi."
  r = ai.json p ext
  audit msg.id r._

  conf = r._.conf
  if conf > 0.85
    auto r c msg                       # high confidence → act + reply
  ef conf >= 0.6
    confirm r c                        # medium → ask owner to confirm
  el
    escalate m c                       # low → full handoff to owner
  rep 200 {ok:true}

# --- routing branches ---

fn auto r c msg
  mt r.intent
    :new_order ->
      o = tools.create_order r.items c.ph r.delivery_date
      tools.send c.ph "Buyurtmangiz qabul qilindi ✅ Jami: ${o.total} so'm. Yetkazish: ${r.delivery_date}."
    :question ->
      a = ai.run "Mijoz savoli: ${msg.body}. Katalog asosida javob ber."
        [tools.get_product_catalog tools.get_customer_history]
      tools.send c.ph a
    :greeting ->
      tools.send c.ph "Assalomu alaykum! Bugun non buyurtma qilasizmi? 🍞"
    :complaint ->
      tools.ask_owner c.owner "Shikoyat ${c.name} dan: ${msg.body}"
      tools.send c.ph "Uzr so'raymiz, egamiz tez orada bog'lanadi."
    _ ->
      tools.send c.ph "Tushundim, rahmat!"

fn confirm r c
  tools.ask_owner c.owner "Tasdiqlang? ${c.name}: ${json.enc r.items} — ${r.delivery_date}. (ha/yo'q)"

fn escalate m c
  tools.ask_owner c.owner "Aniq emas, ko'rib chiqing 👀 ${c.name}: \"${m.text}\""
  tools.send c.ph "Xabaringiz egamizga yuborildi, tez orada javob beramiz."

# --- helpers ---

fn ensure_customer ph name
  c = db.one "select * from customers where ph=$1" [ph]
  if c
    ret c
  owner = db.one "select id from users limit 1"!     # single-tenant default owner
  db.ins "customers" {owner:owner.id name:name ph:ph}

fn audit msg meta
  db.ins "ai_interactions" {msg:msg intent:"" conf:meta.conf tokens:meta.tokens cost:meta.cost ms:meta.ms}
