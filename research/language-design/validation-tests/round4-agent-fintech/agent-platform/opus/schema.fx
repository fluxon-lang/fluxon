# schema.fx — Fluxon tbl schemas for the AI agent platform.
# All persistent state lives here. JSON columns hold tool schemas,
# tool-call lists, tool I/O, and agent memory values.

use db

# An AI agent: owner-scoped, has a system prompt, a model, and a status.
tbl agents
  id            serial pk
  owner         int
  name          str
  system_prompt str
  model         str
  status        sym          # :active :paused :archived
  created       now

# Tools registered against a specific agent. params_schema is JSON.
# handler_kind selects which built-in dispatcher runs the tool
# (:web_search :calculator :get_memory :set_memory :http ...).
tbl tools
  id            serial pk
  agent         int ref:agents.id
  name          str
  description   str
  params_schema json
  handler_kind  sym
  destructive   bool         # destructive tools require confirmation
  created       now

# A conversation between a user and one agent.
# Token/cost totals accumulate here as the agent runs.
tbl conversations
  id            serial pk
  agent         int ref:agents.id
  user_id       str
  total_tokens  int
  total_cost    flt
  created       now

# A single message in a conversation.
# role: :system :user :assistant :tool
# tool_calls is JSON: the list of tool calls the assistant requested.
tbl messages
  id            serial pk
  conversation  int ref:conversations.id
  role          sym
  content       str
  tool_calls    json
  created       now

# One logged tool invocation: input, output, timing, success flag.
tbl tool_invocations
  id            serial pk
  message       int ref:messages.id
  tool_name     str
  input         json
  output        json
  ms            int
  ok            bool
  created       now

# Persistent per-agent key/value memory. value is JSON so the agent
# can store structured data. (agent, key) is logically unique.
tbl agent_memory
  id            serial pk
  agent         int ref:agents.id
  key           str
  value         json
  updated       now

# Pending confirmations for low-confidence or destructive tool calls.
# Surfaced to the user; resumed once approved/denied.
tbl confirmations
  id            serial pk
  conversation  int ref:conversations.id
  agent         int ref:agents.id
  tool_name     str
  input         json
  reason        str
  status        sym          # :pending :approved :denied
  created       now
