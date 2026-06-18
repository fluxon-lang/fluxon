# /model command — switch the AI provider/model at runtime (issue #200).
#
# `ai.config` is not only a top-level setup call — it is the primitive a
# Claude-Code-style `/model` command is built on. Calling it again switches the
# active provider/model/key on the fly, and the NEXT `ai.ask`/`ai.json`/`ai.run`
# uses the new configuration. No restart.
#
# KEY POINT (issue #200): a non-empty `ai.config {..}` is a PARTIAL update. The
# given fields merge on top of what is already set, so `/model` can switch ONLY
# the model and keep the key/url/style. To reset everything back to the env
# defaults, call `ai.config {}` (empty).
#
# This demo uses Anthropic as the base (zero-config via $ANTHROPIC_API_KEY) and
# lets the user pick a model with `/model`. A real app would add OpenAI/GLM
# profiles the same way (style + url + key).
#
#   export ANTHROPIC_API_KEY=sk-ant-...
#   fluxon run examples/ai_model_switch.fx

use ai io

# The models the `/model` command offers. Each entry is just the partial config
# applied when picked — here only `model` changes, but it could carry a full
# {style url key model} to switch provider too.
models = [
  {name: "Opus (smart)"  model: "claude-opus-4-8"}
  {name: "Sonnet (fast)" model: "claude-sonnet-4-6"}
  {name: "Haiku (cheap)" model: "claude-haiku-4-5"}
]

current <- "claude-opus-4-8"

io.print "Fluxon AI chat — '/model' to switch model, 'quit' to exit\n"
io.print "(current model: ${current})\n\n"

history <- []

each i in 1..1000
  question = io.prompt "you> "

  if question == nil
    io.print "\nbye!\n"
    ret nil
  if question == "quit" | question == "exit" | question == "/q"
    io.print "bye!\n"
    ret nil
  if question == ""
    skip

  # /model — list the choices, apply the picked one with a PARTIAL ai.config.
  if question == "/model"
    io.print "available models:\n"
    each m in models
      io.print "  ${m.name} — ${m.model}\n"
    pick = io.prompt "model name (substring)> "
    chosen <- nil
    each m in models
      if pick != nil & pick != "" & str.has m.model pick
        chosen <- m
    if chosen == nil
      io.print "no match — keeping ${current}\n\n"
      skip
    # The whole switch: a partial config that changes ONLY the model. The key
    # (from $ANTHROPIC_API_KEY) and everything else carry over untouched.
    ai.config {model: chosen.model}
    current <- chosen.model
    io.print "switched to ${current}\n\n"
    skip

  history <- history.push {role::user content:question}
  r = ai.run history []
  answer = r.text
  io.print "ai (${current})> ${answer}\n\n"
  history <- history.push {role::assistant content:answer}
