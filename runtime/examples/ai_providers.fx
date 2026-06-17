# AI provider override demo — talk to any OpenAI-compatible API (issue #199).
#
# The `ai` battery works zero-config with a standard key in .env. This example
# shows the ADVANCED escape hatch: pointing it at a non-standard provider
# (Z.AI / GLM, OpenRouter, Ollama, vLLM, ...) by overriding the request shape.
# Default behavior is unchanged — these overrides are purely additive.
#
# Pick ONE provider below and set its key, e.g.:
#   export ZAI_KEY=...        # then: fluxon run examples/ai_providers.fx
#
# NOTE: this calls the real API (spends tokens). Without a key it errors clearly
# and never reaches the network.

use ai

# --- Option A: GLM via Z.AI ---------------------------------------------------
# GLM speaks the OpenAI wire format; only the URL + key differ. `style::openai`
# selects the format, `url` swaps the endpoint — that is the whole change.
ai.config {
  style: :openai
  url:   "https://api.z.ai/api/paas/v4/chat/completions"
  key:   env.ZAI_KEY
  model: "glm-4.6"
}

answer = ai.ask "In one sentence: what is the Fluxon language?"
log "GLM says: ${answer}"

# Per-call opts override the global config for a single call — here, a cheaper
# model for a quick check. Omit opts entirely and the call is unchanged.
quick = ai.ask "Reply with just: OK" {model: "glm-4.5-air"}
log "quick model: ${quick}"

# --- Option B: OpenRouter (extra body params + recommended headers) -----------
# Uncomment to use OpenRouter instead. It accepts vendor-specific body fields
# (`provider`/`route`/...) and recommends HTTP-Referer / X-Title headers — both
# merge onto the defaults. Hyphenated header names must be STRING keys (a bare
# map key cannot contain `-`).
#
# ai.config {
#   style:   :openai
#   url:     "https://openrouter.ai/api/v1/chat/completions"
#   key:     env.OPENROUTER_KEY
#   model:   "anthropic/claude-3.5-sonnet"
#   headers: {"HTTP-Referer": "https://myapp.dev" "X-Title": "Fluxon demo"}
#   extra:   {provider: {sort: "throughput"}}
# }
# log ai.ask "Salom!"

# --- Option C: a local server (Ollama / vLLM / LM Studio) ---------------------
# A local OpenAI-compatible server usually needs only the URL (key can be any
# non-empty string).
#
# ai.config {
#   style: :openai
#   url:   "http://localhost:11434/v1/chat/completions"
#   key:   "local"
#   model: "llama3.1"
# }
# log ai.ask "Hello from a local model"
