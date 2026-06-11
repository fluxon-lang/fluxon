-- Fluxon Notes REST API
-- Endpoints:
--   GET    /notes         list all notes
--   POST   /notes         create note {title, body}
--   GET    /notes/:id     get note by id
--   DELETE /notes/:id     delete note

use http
use db
use json
use env

let PORT = num.parse(env.get("PORT") or "3000")
let DB_URL = env.get("DB_URL") or "notes.db"

-- Init DB connection and schema
db.open(DB_URL)
db.exec!(
  "CREATE TABLE IF NOT EXISTS notes (
    id    INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    body  TEXT NOT NULL DEFAULT '',
    ts    INTEGER NOT NULL
  )"
)

-- Helpers

fn validate_note %body -> $
  if !body.has("title")
    "title is required"
  elif body.title.trim().len() == 0
    "title must not be blank"
  else
    ""

fn row_to_note ~r -> %
  {
    id:    r.id,
    title: r.title,
    body:  r.body,
    ts:    r.ts
  }

-- GET /notes
http.get "/notes" fn req res ->
  @rows = db.query!("SELECT * FROM notes ORDER BY ts DESC")
  @notes = rows |> map(fn ~r -> row_to_note(r))
  res.json({ok: true, notes: notes})

-- POST /notes
http.post "/notes" fn req res ->
  ~body = req.body
  $err = validate_note(body)
  if err.len() > 0
    res.status(400).json({ok: false, error: err})
  else
    #ts = time.now()
    db.exec!(
      "INSERT INTO notes (title, body, ts) VALUES (?, ?, ?)",
      [body.title, body.body or "", ts]
    )
    @row = db.query!("SELECT * FROM notes ORDER BY id DESC LIMIT 1")
    res.status(201).json({ok: true, note: row_to_note(row[0])})

-- GET /notes/:id
http.get "/notes/:id" fn req res ->
  try
    #id = num.parse!(req.params.id)
    @rows = db.query!("SELECT * FROM notes WHERE id = ?", [id])
    if rows.len() == 0
      res.status(404).json({ok: false, error: "note not found"})
    else
      res.json({ok: true, note: row_to_note(rows[0])})
  catch $err
    res.status(400).json({ok: false, error: "invalid id"})

-- DELETE /notes/:id
http.delete "/notes/:id" fn req res ->
  try
    #id = num.parse!(req.params.id)
    @rows = db.query!("SELECT id FROM notes WHERE id = ?", [id])
    if rows.len() == 0
      res.status(404).json({ok: false, error: "note not found"})
    else
      db.exec!("DELETE FROM notes WHERE id = ?", [id])
      res.json({ok: true, deleted: id})
  catch $err
    res.status(400).json({ok: false, error: "invalid id"})

-- 404 fallback
http.notfound fn req res ->
  res.status(404).json({ok: false, error: "route not found"})

-- Start server
show "Notes API on port " + num.str(PORT)
http.serve(PORT)
