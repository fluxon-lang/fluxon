# aifeatures.flux — AI xususiyatlari
# (a) POST /products/:id/summarize — sharhlardan marketing xulosasi.
# (b) GET /recommend/:customer_id — buyurtma tarixidan tavsiyalar.
# Fayl nomi "ai" emas (batareya bilan to'qnashmasin uchun) — aifeatures.
use http db ai

# Ro'yxatdan dastlabki n ta elementni oladi (list slice yo'q, qo'lda quramiz).
fn take lst n
  out <- []
  i <- 0
  each x in lst
    if i < n
      out <- out.push x
    i <- i + 1
  ret out

# POST /products/:id/summarize — mahsulot sharhlaridan marketing matni.
http.on :post "/products/:id/summarize" \req ->
  pid = req.params.id
  prod = db.one "select * from products where id=$1" [pid]
  if prod == nil
    rep 404 {error:"product not found"}
  else
    reviews = db.q "select rating, body from reviews where product=$1 order by created desc limit 50" [pid]
    if reviews.len == 0
      rep 200 {product:pid summary:"Hali sharhlar yo'q." based_on:0}
    else
      # Sharhlarni bitta matn blokiga yig'amiz.
      blob <- ""
      each r in reviews
        blob <- blob + "- (${r.rating}/5) ${r.body}\n"
      summary = ai.ask "Quyidagi mijoz sharhlari asosida '${prod.name}' mahsuloti uchun qisqa, jozibali marketing xulosasi yoz. Faqat xulosa matnini qaytar.\n\nSharhlar:\n${blob}"
      rep 200 {product:pid summary:summary based_on:reviews.len}

# GET /recommend/:customer_id — buyurtma tarixiga asoslangan tavsiyalar.
http.on :get "/recommend/:customer_id" \req ->
  cid = req.params.customer_id
  cust = db.one "select * from customers where id=$1" [cid]
  if cust == nil
    rep 404 {error:"customer not found"}
  else
    # Mijoz sotib olgan mahsulotlar tarixini olamiz.
    history = db.q "select p.name, p.category, oi.qty from order_items oi join orders o on o.id = oi.order join products p on p.id = oi.product where o.customer=$1 order by o.created desc limit 50" [cid]

    # Joriy katalog (zaxirada bor mahsulotlar).
    catalog = db.q "select id, name, category, price from products where stock > 0 order by id limit 100"

    if history.len == 0
      # Tarix yo'q — eng yangi katalogdan qaytaramiz.
      rep 200 {customer:cid based_on:"no history" recommendations:(take catalog 5)}
    else
      # Tarix matnini quramiz.
      hist_blob <- ""
      each h in history
        hist_blob <- hist_blob + "- ${h.name} (${h.category}) x${h.qty}\n"

      cat_blob <- ""
      each c in catalog
        cat_blob <- cat_blob + "id=${c.id} | ${c.name} | ${c.category} | ${c.price}\n"

      # AI dan strukturali tavsiya so'raymiz.
      r = ai.json "Mijozning xarid tarixi asosida katalogdan 3 ta mos mahsulot tavsiya qil. Faqat katalogdagi id larni ishlat.\n\nTarix:\n${hist_blob}\nKatalog:\n${cat_blob}" {
        recommendations: [{product_id:int reason:str}]
      }

      # AI ishonchiga qarab marshrutlash.
      if r._.conf > 0.6
        rep 200 {customer:cid based_on:"order history" confidence:r._.conf recommendations:r.recommendations}
      else
        # Past ishonch — xavfsiz fallback (katalogdan).
        rep 200 {customer:cid based_on:"fallback (low confidence)" confidence:r._.conf recommendations:(take catalog 3)}
