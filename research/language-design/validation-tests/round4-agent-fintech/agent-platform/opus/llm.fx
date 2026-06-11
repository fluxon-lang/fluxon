# llm.fx — LLM access for the agent loop.
#
# We implement a MANUAL tool-calling loop with ai.json rather than
# leaning on ai.run, because ai.run (per spec) takes a list of Fluxon
# FUNCTIONS and runs the loop opaquely — it doesn't let us (a) inject
# per-agent persistent memory, (b) log every invocation with timing,
# (c) gate destructive/low-confidence calls behind confirmation, or
# (d) accumulate cost per conversation. So we drive the loop ourselves
# and ask the model for a STRUCTURED decision each turn. (See spec-gaps.)
#
# Aliased to `llm` so callers don't collide with the `ai` battery.

use ai json

# Ask the model for its next step given the running transcript and the
# tool catalog. The schema forces one of two shapes: call a tool, or
# answer. We pass tool descriptions as text so the model knows options.
exp fn decide system_prompt memory_text tools_text transcript_text
  prompt = "You are an AI agent.
System prompt:
${system_prompt}

Your persistent memory:
${memory_text}

Tools you may call (name — description — params):
${tools_text}

Conversation so far:
${transcript_text}

Decide the single next step. If you need a tool, set action to :call
and fill tool + input. If you can answer the user, set action to :final
and fill answer. Set confidence 0..1 for how sure you are."
  r = ai.json prompt {
    action: ":call|:final"
    tool: "str"
    input: "map"
    answer: "str"
    confidence: "flt"
    reasoning: "str"
  }
  ret r

# Extract usage metadata from any ai.* result. The spec says every
# ai.* result carries `_` with .tokens .cost .ms .conf. We coalesce
# defensively in case a field is absent.
exp fn usage r
  meta = r._ ?? {}
  ret {
    tokens:(meta.tokens ?? 0)
    cost:(meta.cost ?? 0.0)
    ms:(meta.ms ?? 0)
    conf:(meta.conf ?? 0.0)
  }

# A plain text completion (used for fallbacks / summaries).
exp fn ask prompt
  ret ai.ask prompt
