use ai json

# Score a candidate match against a job using AI
exp fn score_match job candidate
  prompt = "Harmonitalarni 0 va 1 orasida baholang (0.0-1.0). Javob shunday bo'lsin:\n{\n  \"score\": <raqam>,\n  \"reasons\": \"<tafsilot>\"\n}\n\nJish:\nTitle: ${job.title}\nDescription: ${job.description}\nSalary: ${job.salary_min} - ${job.salary_max}\n\nCandidate:\nName: ${candidate.name}\nSkills: ${candidate.skills}\nResume: ${candidate.resume}"

  result = ai.json prompt {
    score:flt
    reasons:str
  }

  score = result.score
  if !score
    score = 0.0

  ret {
    score:score
    reasons:result.reasons
    confidence:result._.conf
  }

# Determine application status based on match score
fn determine_status score
  if score > 0.85
    ret :shortlisted
  elif score >= 0.6
    ret :review
  else
    ret :rejected

# Get AI confidence explanation
fn confidence_reason conf
  if conf > 0.85
    ret "High confidence AI matching"
  elif conf >= 0.6
    ret "Medium confidence matching"
  else
    ret "Low confidence - manual review recommended"
