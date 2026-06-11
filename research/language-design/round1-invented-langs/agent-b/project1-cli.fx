-- Fluxon TODO Manager
-- Usage: fluxon todo <add|list|done|del> [args]

use fs
use json
use args

let FILE = "todos.json"

fn load -> @
  ?exists = fs.exists(FILE)
  if exists
    $raw = fs.read!(FILE)
    json.parse!(raw)
  else
    []

fn save @todos
  fs.write!(FILE, json.encode(todos))

fn next_id @todos -> #
  if todos.len() == 0
    1
  else
    ~last = todos[todos.len() - 1]
    last.id + 1

fn cmd_add $text @todos
  #id = next_id(todos)
  %todo = {id: id, text: text, done: false}
  todos.push(todo)
  save(todos)
  show "Added #" + num.str(id) + ": " + text

fn cmd_list @todos
  if todos.len() == 0
    show "No todos yet."
  else
    each %t in todos
      $mark = if t.done then "[x]" else "[ ]"
      show num.str(t.id) + " " + mark + " " + t.text

fn cmd_done #id @todos -> ?
  ?found = false
  each %t in todos
    if t.id == id
      t.done = true
      ?found = true
  if found
    save(todos)
    show "Marked #" + num.str(id) + " done."
  else
    show "Todo #" + num.str(id) + " not found."
  found

fn cmd_del #id @todos -> ?
  #before = todos.len()
  @kept = todos.filter(fn ~t -> t.id != id)
  if kept.len() < before
    save(kept)
    show "Deleted #" + num.str(id) + "."
    true
  else
    show "Todo #" + num.str(id) + " not found."
    false

fn usage
  show "Usage:"
  show "  todo add <text>    -- add a new todo"
  show "  todo list          -- list all todos"
  show "  todo done <id>     -- mark todo complete"
  show "  todo del <id>      -- delete a todo"

-- Main entry point
$cmd = args[0] or ""
@todos = load()

match cmd
  "add" ->
    $text = args[1..] |> join(" ")
    if text.trim().len() == 0
      show "Error: todo text required."
      fail "missing text"
    cmd_add(text, todos)

  "list" ->
    cmd_list(todos)

  "done" ->
    ~raw = args[1]
    if raw == nil
      show "Error: id required."
      fail "missing id"
    #id = num.parse!(raw)
    cmd_done(id, todos)

  "del" ->
    ~raw = args[1]
    if raw == nil
      show "Error: id required."
      fail "missing id"
    #id = num.parse!(raw)
    cmd_del(id, todos)

  _ ->
    usage()
