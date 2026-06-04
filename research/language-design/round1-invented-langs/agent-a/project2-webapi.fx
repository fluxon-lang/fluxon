# notes-api.fx — REST API for a notes app, backed by sqlite.
use list
use str

db = @db("sqlite://notes.db")
db.run("create table if not exists notes(id text primary key, title text, body text, ts int)")

srv = @web()

# GET /notes -> all notes
srv.get("/notes", \req:
  {json: db.q("select * from notes order by ts desc")})

# GET /notes/:id -> one note or 404
srv.get("/notes/:id", \req:
  rows = db.q("select * from notes where id=?", req.params.id)
  ? list.len(rows) == 0:
    {status: 404, json: {error: "not found"}}
  |:
    {json: rows[0]})

# POST /notes -> create, with validation
srv.post("/notes", \req:
  b = req.body
  title = ? b == nil: nil |: b.title
  body  = ? b == nil: nil |: b.body
  ? title == nil or str.trim("$title") == "":
    ret {status: 400, json: {error: "title required"}}
  ? body == nil:
    ret {status: 400, json: {error: "body required"}}
  note = {id: @uid(), title: title, body: body, ts: @now()}
  db.q("insert into notes(id,title,body,ts) values(?,?,?,?)", note.id, note.title, note.body, note.ts)
  {status: 201, json: note})

# DELETE /notes/:id
srv.del("/notes/:id", \req:
  rows = db.q("select id from notes where id=?", req.params.id)
  ? list.len(rows) == 0:
    {status: 404, json: {error: "not found"}}
  |:
    db.q("delete from notes where id=?", req.params.id)
    {status: 204})

say "notes API on :${@env("PORT", "8080")}"
srv.run(str.int(@env("PORT", "8080")))
