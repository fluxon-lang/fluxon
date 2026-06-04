use http db

# Create a new candidate profile
exp fn create_candidate req
  body = req.body

  if !body.name
    ret rep 400 {error:"name kerak"}
  if !body.email
    ret rep 400 {error:"email kerak"}
  if !body.skills
    ret rep 400 {error:"skills kerak"}

  candidate = db.ins "candidates" {
    name:body.name
    email:body.email
    skills:body.skills
    resume:(body.resume ?? "")
  }
  rep 201 candidate

# Get candidate profile
exp fn get_candidate req
  cid = str.int req.params.id
  candidate = db.one "select * from candidates where id=$1" [cid]
  if !candidate
    ret rep 404 {error:"nomzod topilmadi"}
  rep 200 candidate

# Update candidate profile
exp fn update_candidate req
  cid = str.int req.params.id
  body = req.body

  # Verify candidate exists
  candidate = db.one "select id from candidates where id=$1" [cid]!

  updates = {email:(body.email ?? nil)}

  if body.skills
    updates = updates.set "skills" body.skills
  if body.resume
    updates = updates.set "resume" body.resume
  if body.name
    updates = updates.set "name" body.name

  db.up "candidates" updates {id:cid}
  updated = db.one "select * from candidates where id=$1" [cid]
  rep 200 updated

# List all candidates
exp fn list_candidates req
  candidates = db.q "select * from candidates order by created desc"
  rep 200 candidates
