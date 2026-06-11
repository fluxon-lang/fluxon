# tools_builtin.fx — built-in tools every agent can invoke by name
# Exported as plain Fluxon functions; the runtime dispatches to them by name.

use http db
use ./memory

# ── web_search — stub via http.get to a search API ───────────────────────────
# SPEC GAP: no native search; we stub to a hypothetical REST endpoint.
# In production replace with a real search provider URL + API key.
exp fn builtin_web_search input
  query = input.query ?? ""
  if query == ""
    ret {ok:false error:"query required"}
  encoded = query  # SPEC GAP: no url-encode fn in spec — assume simple queries
  res = http.get "https://api.search.example.com/search?q=${encoded}&limit=5"
  if res.status == 200
    ret {ok:true results:res.body.results ?? []}
  ret {ok:false error:"search failed status=${res.status}"}

# ── calculator — evaluate simple arithmetic expressions ──────────────────────
# SPEC GAP: no eval() or expression parser in spec.
# We handle the four common operations only via structured input.
# Input: {op: "+"|"-"|"*"|"/", a: number, b: number}
exp fn builtin_calculator input
  a = input.a ?? 0
  b = input.b ?? 0
  op = input.op ?? "+"
  if op == "+"
    ret {ok:true result:a + b}
  elif op == "-"
    ret {ok:true result:a - b}
  elif op == "*"
    ret {ok:true result:a * b}
  elif op == "/"
    if b == 0
      ret {ok:false error:"division by zero"}
    ret {ok:true result:a / b}
  ret {ok:false error:"unknown op ${op}"}

# ── get_memory — retrieve a single memory key for an agent ───────────────────
# Input: {agent_id: int, key: str}
exp fn builtin_get_memory input
  agent_id = input.agent_id
  key = input.key ?? ""
  if key == ""
    ret {ok:false error:"key required"}
  val = memory.mem_get agent_id key
  if val == nil
    ret {ok:true found:false value:nil}
  ret {ok:true found:true value:val}

# ── set_memory — write a memory value for an agent ───────────────────────────
# Input: {agent_id: int, key: str, value: any}
exp fn builtin_set_memory input
  agent_id = input.agent_id
  key = input.key ?? ""
  value = input.value
  if key == ""
    ret {ok:false error:"key required"}
  memory.mem_set agent_id key value
  ret {ok:true stored:true}

# ── Master dispatch table (name → fn) ─────────────────────────────────────────
# SPEC GAP: Fluxon has no first-class function map / function references in a map.
# We model the dispatch table as a plain data map keyed by tool name string,
# but since map values must be literals (not fn refs), we use a helper fn
# that switches on the name. See tools_dispatch below.

exp fn dispatch_builtin name input
  match name
    "web_search"   -> builtin_web_search input
    "calculator"   -> builtin_calculator input
    "get_memory"   -> builtin_get_memory input
    "set_memory"   -> builtin_set_memory input
    _              -> {ok:false error:"unknown builtin ${name}"}

# ── Catalog of built-in tool descriptors (for prompt injection) ───────────────
exp builtin_catalog = [
  {
    name:"web_search"
    description:"Search the web for a query. Returns a list of results."
    params_schema:{
      type:"object"
      properties:{query:{type:"string" description:"The search query"}}
      required:["query"]
    }
  }
  {
    name:"calculator"
    description:"Evaluate arithmetic. Provide op (+,-,*,/), a, and b."
    params_schema:{
      type:"object"
      properties:{
        op:{type:"string" enum:["+","-","*","/"]}
        a:{type:"number"}
        b:{type:"number"}
      }
      required:["op","a","b"]
    }
  }
  {
    name:"get_memory"
    description:"Retrieve a stored memory value by key for this agent."
    params_schema:{
      type:"object"
      properties:{
        agent_id:{type:"integer"}
        key:{type:"string"}
      }
      required:["agent_id","key"]
    }
  }
  {
    name:"set_memory"
    description:"Store a value in persistent agent memory under a key."
    params_schema:{
      type:"object"
      properties:{
        agent_id:{type:"integer"}
        key:{type:"string"}
        value:{}
      }
      required:["agent_id","key","value"]
    }
  }
]
