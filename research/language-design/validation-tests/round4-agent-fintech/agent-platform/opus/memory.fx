# memory.fx — persistent, mutable per-agent memory.
# Memory survives across conversations: it is a row in agent_memory,
# keyed by (agent, key). value is a JSON column so we store any shape.
#
# NOTE (spec gap): the spec has no UPSERT helper and `db.up`/`db.ins`
# don't return "was it inserted?". We emulate upsert with a read-then
# write, wrapped in db.tx so a concurrent run can't double-insert.

use db json time

# Read one memory value. Returns the decoded JSON value or nil.
exp fn get agent_id key
  row = db.one "select * from agent_memory where agent=$1 and key=$2" [agent_id key]
  if !row
    ret nil
  # JSON columns come back already decoded into Fluxon values (assumption,
  # documented in spec-gaps). If it's a raw string, json.dec handles it.
  ret row.value

# Write/overwrite one memory value (JSON-encodable).
exp fn set agent_id key val
  db.tx \->
    existing = db.one "select id from agent_memory where agent=$1 and key=$2" [agent_id key]
    if existing
      db.up "agent_memory" {value:val updated:time.now} {id:existing.id}
      ret val
    db.ins "agent_memory" {agent:agent_id key:key value:val}
    ret val

# Delete a memory key. Returns :ok.
exp fn drop agent_id key
  db.del "agent_memory" {agent:agent_id key:key}
  ret :ok

# All memory for an agent as a flat map {key: value}. Used to inject
# memory into the system prompt at the start of a run.
exp fn dump agent_id
  rows = db.q "select key, value from agent_memory where agent=$1 order by key" [agent_id]
  out <- {}
  each r in rows
    out <- out.set r.key r.value
  ret out

# A compact human-readable rendering of memory for the LLM prompt.
exp fn render agent_id
  m = dump agent_id
  if m.keys.len == 0
    ret "(no stored memory)"
  lines <- []
  each k, v in m
    lines <- lines.push "- ${k}: ${json.enc v}"
  ret lines.join "\n"
