use http db ai json

# Create a new job posting
exp fn create_job req
  body = req.body
  cid = str.int req.params.company_id

  if !body.title
    ret rep 400 {error:"title kerak"}
  if !body.description
    ret rep 400 {error:"description kerak"}
  if !body.salary_min
    ret rep 400 {error:"salary_min kerak"}
  if !body.salary_max
    ret rep 400 {error:"salary_max kerak"}

  # Verify company exists
  company = db.one "select id from companies where id=$1" [cid]!

  job = db.ins "jobs" {
    company_id:cid
    title:body.title
    description:body.description
    salary_min:body.salary_min
    salary_max:body.salary_max
    status::open
  }
  rep 201 job

# List jobs with filtering and pagination
exp fn list_jobs req
  status_filter = req.query.status ?? ":open"
  search_term = req.query.search ?? ""
  page = str.int (req.query.page ?? "1")
  limit = 20
  offset = (page - 1) * limit

  where_clause = "status=$1"
  params = [status_filter]

  if search_term != ""
    where_clause = where_clause + " and (title ilike $2 or description ilike $2)"
    params = params.push ("%${search_term}%")

  query = "select j.*, c.name company_name from jobs j join companies c on j.company_id=c.id where ${where_clause} order by j.created desc limit ${limit} offset ${offset}"

  jobs = db.q query params
  rep 200 jobs

# Get single job by ID
exp fn get_job req
  jid = str.int req.params.id
  job = db.one "select j.*, c.name company_name from jobs j join companies c on j.company_id=c.id where j.id=$1" [jid]
  if !job
    ret rep 404 {error:"ish topilmadi"}
  rep 200 job

# Summarize a job (AI feature)
exp fn summarize_job req
  jid = str.int req.params.id

  job = db.one "select * from jobs where id=$1" [jid]!

  # Check if summary already exists
  existing = db.one "select * from application_summaries where job_id=$1" [jid]
  if existing
    ret rep 200 {summary:existing.summary}

  # Use AI to generate summary
  prompt = "Ushbu ish tavsifi asosida qisqa, nomzodlarga qaratilgan xulosa yozing (2-3 jumlada):\n\n${job.description}"
  summary = ai.ask prompt

  # Store summary
  db.ins "application_summaries" {
    job_id:jid
    summary:summary
  }

  rep 200 {summary:summary}
