use ai json
use ./models

# Generate AI summary of poll results using ai.ask
exp fn summarize_poll poll_id
  results = models.get_poll_results poll_id

  # Build prompt with results
  options_text <- ""
  each opt in results.options
    line = "${opt.text}: ${opt.votes} votes"
    options_text <- options_text + line + "\n"

  prompt = "Poll sualiga javoblarni qisqacha tahlil qil: \"${results.question}\"\n\nNatijalari:\n${options_text}\n\nQaysi variant yutdi va qancha foiz bilan? Qisqacha xulosani ber."

  summary = ai.ask prompt

  ret {
    poll_id:poll_id
    question:results.question
    summary:summary
  }

# Generate JSON-structured AI analysis with confidence
exp fn analyze_poll_json poll_id
  results = models.get_poll_results poll_id

  # Build prompt for structured output
  options_text <- ""
  each opt in results.options
    line = "${opt.text}: ${opt.votes}"
    options_text <- options_text + line + "\n"

  prompt = "Quyidagi poll natijalari asosida tahlil et: ${results.question}. Natijalari: ${options_text}. JSON bilan: {winner:str votes:int percentage:flt summary:str}"

  r = ai.json prompt {
    winner:str
    votes:int
    percentage:flt
    summary:str
  }

  if r._.conf > 0.85
    ret {
      status::confident
      analysis:r
    }
  elif r._.conf >= 0.6
    ret {
      status::moderate
      analysis:r
    }
  else
    ret {
      status::low
      analysis:r
    }
