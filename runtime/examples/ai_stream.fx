# ai.stream — token-by-token streaming (issue #201).
#
# `ai.ask` blocks until the whole answer is ready. `ai.stream` delivers the
# answer in CHUNKS as the model generates them: the callback fires for each text
# chunk (the "typing" effect), and the full accumulated text is returned at the
# end — so you can still add it to a conversation history like `ai.ask`.
#
#   text = ai.stream "prompt" \chunk -> io.print chunk   # print as it streams
#   text = ai.stream "prompt" \chunk -> ws.send sid chunk  # relay to a ws client
#
# Zero-config: the provider/key are auto-detected (ANTHROPIC_API_KEY / OPENAI_API_KEY),
# same as ai.ask. A trailing opts map works too (ai.stream p cb {model:"..."}).
#
#   export ANTHROPIC_API_KEY=sk-ant-...
#   fluxon run examples/ai_stream.fx

use ai io

io.print "Streaming chat — 'quit' to exit\n\n"

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

  history <- history.push {role::user content:question}

  io.print "ai > "
  # The callback prints each chunk as it arrives (io.print does not add a
  # newline) — the answer renders progressively. ai.stream returns the full text.
  answer = ai.stream question \chunk -> io.print chunk
  io.print "\n\n"

  history <- history.push {role::assistant content:answer}
