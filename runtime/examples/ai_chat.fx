# AI chat — a small terminal conversation (REPL).
#
# The key is found AUTOMATICALLY (nothing to configure):
#   if ANTHROPIC_API_KEY is in .env or the environment -> Claude
#   if OPENAI_API_KEY                                  -> GPT
# Model: $AI_MODEL ?? provider default. Override: $AI_PROVIDER (anthropic|openai).
#
# Running it — fluxon finds the `.env` from WHICHEVER directory it is
# started in (env_lookup reads the `.env` in the current directory).
#   # if .env is in the project root:
#   cd <project-root> && runtime/target/release/fluxon run runtime/examples/ai_chat.fx
#   # or export the key to the environment and run from any directory:
#   export ANTHROPIC_API_KEY=sk-ant-...   # or OPENAI_API_KEY=sk-...
#   fluxon run examples/ai_chat.fx
#
# To exit: type "quit", "exit" or "/q" (or Ctrl+D).

use ai io

io.print "Fluxon AI chat — type your question ('quit' = finish)\n\n"

# Conversation history — every new message and reply is added here, so the model
# remembers the context (multi-step conversation via ai.run msgs).
history <- []

each i in 1..1000
  question = io.prompt "you> "

  # EOF (Ctrl+D) -> nil; or exit words -> we finish.
  if question == nil
    io.print "\nbye!\n"
    ret nil
  if question == "quit" | question == "exit" | question == "/q"
    io.print "bye!\n"
    ret nil
  # skip an empty line.
  if question == ""
    skip

  # Add the user message to the history.
  history <- history.push {role::user content:question}

  # Reply from the model — ai.run returns a single step. There is no tool in
  # this chat, so it always returns :final.
  r = ai.run history []

  answer = r.text
  io.print "ai > ${answer}\n\n"

  # Add the model reply to the history too (for context in the next question).
  history <- history.push {role::assistant content:answer}
