# Job Board Platform — Schema Definitions

tbl companies
  id           serial pk
  name         str
  website      str
  description  str
  created      now

tbl jobs
  id           serial pk
  company_id   int ref:companies.id
  title        str
  description  str
  salary_min   money
  salary_max   money
  status       sym
  created      now

tbl candidates
  id           serial pk
  name         str
  email        str
  skills       str
  resume       str
  created      now

tbl applications
  id           serial pk
  job_id       int ref:jobs.id
  candidate_id int ref:candidates.id
  status       sym
  match_score  flt
  created      now

tbl notifications
  id           serial pk
  user_id      int
  body         str
  read         bool
  created      now

tbl application_summaries
  id           serial pk
  job_id       int ref:jobs.id uniq
  summary      str
  created      now
