use http db json
use ./matching as match_mod
use ./realtime as rt

# Apply to a job - atomic operation with notification
exp fn apply_to_job req
  body = req.body
  jid = str.int req.params.job_id

  # Validate required fields
  if !body.candidate_id
    ret rep 400 {error:"candidate_id kerak"}

  cand_id = body.candidate_id

  # Fetch job and candidate
  job = db.one "select * from jobs where id=$1" [jid]!
  candidate = db.one "select * from candidates where id=$1" [cand_id]!

  # Atomic transaction: create application + score + create notification
  result = db.tx \->
    # Check for duplicate application
    existing = db.one "select id from applications where job_id=$1 and candidate_id=$2" [jid cand_id]
    if existing
      fail 409 "Siz bu ishga allaqachon ariza qo'yishgan"

    # Score the match
    match_result = match_mod.score_match job candidate
    status = match_mod.determine_status match_result.score

    # Create application
    application = db.ins "applications" {
      job_id:jid
      candidate_id:cand_id
      status:status
      match_score:match_result.score
    }

    # Create notification for candidate
    notif_body = "Sizning '${job.title}' ishi uchun arizangiz ko'rilmoqda. Matching ball: ${match_result.score}"
    rt.create_notification cand_id notif_body

    ret {
      application:application
      match_result:match_result
      status:status
    }

  rep 201 result

# Get applications for a candidate
exp fn get_candidate_applications req
  cand_id = str.int req.params.candidate_id
  applications = db.q "
    select
      a.*,
      j.title job_title,
      j.company_id,
      c.name company_name
    from applications a
    join jobs j on a.job_id = j.id
    join companies c on j.company_id = c.id
    where a.candidate_id = $1
    order by a.created desc
  " [cand_id]
  rep 200 applications

# Get applications for a job
exp fn get_job_applications req
  jid = str.int req.params.job_id
  applications = db.q "
    select
      a.*,
      c.name candidate_name,
      c.email,
      c.skills
    from applications a
    join candidates c on a.candidate_id = c.id
    where a.job_id = $1
    order by a.match_score desc, a.created desc
  " [jid]
  rep 200 applications

# Update application status (e.g., from review to shortlisted)
exp fn update_application_status req
  app_id = str.int req.params.id
  body = req.body
  new_status = body.status

  if !new_status
    ret rep 400 {error:"status kerak"}

  # Validate status symbol
  match new_status
    :shortlisted -> log "valid"
    :review -> log "valid"
    :rejected -> log "valid"
    :accepted -> log "valid"
    _ -> ret rep 400 {error:"noto'g'ri status"}

  # Get application
  app = db.one "select * from applications where id=$1" [app_id]!

  # Update and notify
  db.tx \->
    db.up "applications" {status:new_status} {id:app_id}

    # Notify candidate
    job = db.one "select title from jobs where id=$1" [app.job_id]
    status_text = match new_status
      :shortlisted -> "Tabriklaymiz! Siz qisqartirildi"
      :review -> "Sizning ariza ko'rib chiqilmoqda"
      :rejected -> "Rahbariyamiz, siz olib tashlandi"
      :accepted -> "Tabriklaymiz! Siz qabul qilindilar"
      _ -> "Status o'zgartirildi"

    notif_body = "'${job.title}' ishi uchun: ${status_text}"
    rt.create_notification app.candidate_id notif_body

  rep 200 {status:new_status}

# Get application details
exp fn get_application req
  app_id = str.int req.params.id
  app = db.one "
    select
      a.*,
      j.title job_title,
      j.description,
      c.name candidate_name,
      c.email,
      c.skills
    from applications a
    join jobs j on a.job_id = j.id
    join candidates c on a.candidate_id = c.id
    where a.id = $1
  " [app_id]
  if !app
    ret rep 404 {error:"ariza topilmadi"}
  rep 200 app
