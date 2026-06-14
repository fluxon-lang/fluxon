# aifeatures.fluxon — AI features
# (a) POST /products/:id/summarize — a marketing summary from reviews.
# (b) GET /recommend/:customer_id — recommendations from order history.
# The file is named "aifeatures" (not "ai") to avoid clashing with the battery.
use http db ai

# Take the first n elements of a list (no list slice yet, so build it by hand).
fn take lst n
  out <- []
  i <- 0
  each x in lst
    if i < n
      out <- out.push x
    i <- i + 1
  ret out

# POST /products/:id/summarize — marketing copy from product reviews.
http.on :post "/products/:id/summarize" \req ->
  pid = req.params.id
  prod = db.one "select * from products where id=$1" [pid]
  if prod == nil
    rep 404 {error:"product not found"}
  else
    reviews = db.q "select rating, body from reviews where product=$1 order by created desc limit 50" [pid]
    if reviews.len == 0
      rep 200 {product:pid summary:"No reviews yet." based_on:0}
    else
      # Collect the reviews into a single text block.
      blob <- ""
      each r in reviews
        blob <- blob + "- (${r.rating}/5) ${r.body}\n"
      summary = ai.ask "Based on the following customer reviews, write a short, compelling marketing summary for the product '${prod.name}'. Return only the summary text.\n\nReviews:\n${blob}"
      rep 200 {product:pid summary:summary based_on:reviews.len}

# GET /recommend/:customer_id — recommendations based on order history.
http.on :get "/recommend/:customer_id" \req ->
  cid = req.params.customer_id
  cust = db.one "select * from customers where id=$1" [cid]
  if cust == nil
    rep 404 {error:"customer not found"}
  else
    # Fetch the history of products the customer has bought.
    history = db.q "select p.name, p.category, oi.qty from order_items oi join orders o on o.id = oi.order join products p on p.id = oi.product where o.customer=$1 order by o.created desc limit 50" [cid]

    # Current catalog (products in stock).
    catalog = db.q "select id, name, category, price from products where stock > 0 order by id limit 100"

    if history.len == 0
      # No history — return the newest catalog entries.
      rep 200 {customer:cid based_on:"no history" recommendations:(take catalog 5)}
    else
      # Build the history text.
      hist_blob <- ""
      each h in history
        hist_blob <- hist_blob + "- ${h.name} (${h.category}) x${h.qty}\n"

      cat_blob <- ""
      each c in catalog
        cat_blob <- cat_blob + "id=${c.id} | ${c.name} | ${c.category} | ${c.price}\n"

      # Ask the AI for a structured recommendation.
      r = ai.json "Based on the customer's purchase history, recommend 3 matching products from the catalog. Use only ids that appear in the catalog.\n\nHistory:\n${hist_blob}\nCatalog:\n${cat_blob}" {
        recommendations: [{product_id:int reason:str}]
      }

      # Route based on the AI's confidence.
      if r._.conf > 0.6
        rep 200 {customer:cid based_on:"order history" confidence:r._.conf recommendations:r.recommendations}
      else
        # Low confidence — safe fallback (from the catalog).
        rep 200 {customer:cid based_on:"fallback (low confidence)" confidence:r._.conf recommendations:(take catalog 3)}
