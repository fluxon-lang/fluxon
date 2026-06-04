# todo.fx — command-line TODO manager. Persists to ./todos.json
use list
use str

FILE = "todos.json"

fn load:
  ? not @fs.exists(FILE): ret []
  ret @json.dec(@fs.read(FILE))

fn save todos:
  @fs.write(FILE, @json.enc(todos))

fn nextid todos:
  m = 0
  @@ t in todos:
    ? t.id > m: m = t.id
  ret m + 1

fn cmd_add todos *words:
  text = str.join(words, " ")
  ? text == "": ! "add: text required"
  id = nextid(todos)
  list.push(todos, {id: id, text: text, done: F})
  save(todos)
  say "added #$id: $text"

fn cmd_list todos:
  ? list.len(todos) == 0:
    say "no todos"
    ret nil
  @@ t in todos:
    box = ? t.done: "[x]" |: "[ ]"
    say "$box #${t.id} ${t.text}"

fn find todos id:
  @@ t in todos:
    ? t.id == id: ret t
  ! "no todo #$id"

fn cmd_done todos id:
  t = find(todos, id)
  t.done = T
  save(todos)
  say "completed #$id"

fn cmd_del todos id:
  @@ i, t in todos:
    ? t.id == id:
      list.del(todos, i)
      save(todos)
      say "deleted #$id"
      ret nil
  ! "no todo #$id"

fn usage:
  say "usage: todo <add TEXT | list | done ID | del ID>"

# --- entry ---
args = @args
? list.len(args) == 0:
  usage()
  ret nil

todos = load()
cmd = args[0]
rest = args[1..list.len(args) - 1]

?!:
  ? cmd == "add":
    cmd_add(todos, *rest)
  | cmd == "list":
    cmd_list(todos)
  | cmd == "done":
    cmd_done(todos, str.int(rest[0]))
  | cmd == "del":
    cmd_del(todos, str.int(rest[0]))
  |:
    usage()
|! e:
  say "error: ${e.msg}"
