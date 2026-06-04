use http db time cron json

# Schema for bookmarks table
tbl bookmarks
  id    serial pk
  url   str uniq
  title str
  tags  str
  created now
  clicks int

# POST /bookmarks — Create a new bookmark
http.on :post "/bookmarks" \req ->
  # Validate URL is non-empty
  if !req.body.url || str.len req.body.url == 0
    ret rep 400 {error:"url is required"}

  url = req.body.url
  title = req.body.title ?? ""
  tags = req.body.tags ?? ""

  # Insert bookmark and return created record
  bookmark = db.ins "bookmarks" {url:url title:title tags:tags clicks:0}
  rep 201 bookmark

# GET /bookmarks — List all bookmarks with optional tag filter
http.on :get "/bookmarks" \req ->
  tag = req.query.tag

  if tag && str.len tag > 0
    # Filter by tag: tags field contains the tag string
    rows = db.q "select * from bookmarks where tags like $1 order by created desc" ["%" + tag + "%"]
  else
    # Get all bookmarks
    rows = db.q "select * from bookmarks order by created desc"

  rep 200 rows

# GET /bookmarks/:id — Get a specific bookmark
http.on :get "/bookmarks/:id" \req ->
  id = str.int req.params.id
  bookmark = db.one "select * from bookmarks where id=$1" [id]

  if !bookmark
    ret rep 404 {error:"bookmark not found"}

  rep 200 bookmark

# DELETE /bookmarks/:id — Delete a bookmark
http.on :del "/bookmarks/:id" \req ->
  id = str.int req.params.id

  # Check if bookmark exists
  bookmark = db.one "select * from bookmarks where id=$1" [id]
  if !bookmark
    ret rep 404 {error:"bookmark not found"}

  # Delete it
  db.del "bookmarks" {id:id}
  rep 204 nil

# POST /bookmarks/:id/visit — Increment click counter
http.on :post "/bookmarks/:id/visit" \req ->
  id = str.int req.params.id

  # Get current bookmark
  bookmark = db.one "select * from bookmarks where id=$1" [id]
  if !bookmark
    ret rep 404 {error:"bookmark not found"}

  # Increment clicks in transaction
  result = db.tx \->
    newClicks = bookmark.clicks + 1
    db.up "bookmarks" {clicks:newClicks} {id:id}
    ret db.one "select * from bookmarks where id=$1" [id]

  rep 200 {id:result.id clicks:result.clicks}

# Daily cron at 09:00 — Log stats
cron.dy 9 0 \hour minute ->
  total = db.one "select count(*) c from bookmarks"
  totalCount = total.c ?? 0

  mostClicked = db.one "select id, title, url, clicks from bookmarks order by clicks desc limit 1"

  log "=== Daily Bookmark Stats ==="
  log "Total bookmarks: ${totalCount}"

  if mostClicked && mostClicked.clicks > 0
    log "Most clicked: ${mostClicked.title} (${mostClicked.clicks} clicks)"
  else
    log "Most clicked: none"

# Start server on port 8080
http.serve 8080
