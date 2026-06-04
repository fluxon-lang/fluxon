use http db

# Create a new company
exp fn create_company req
  body = req.body
  if !body.name
    ret rep 400 {error:"name kerak"}
  if !body.website
    ret rep 400 {error:"website kerak"}

  company = db.ins "companies" {
    name:body.name
    website:body.website
    description:(body.description ?? "")
  }
  rep 201 company

# Get all companies
exp fn list_companies req
  companies = db.q "select * from companies order by created desc"
  rep 200 companies

# Get company by ID
exp fn get_company req
  cid = str.int req.params.id
  company = db.one "select * from companies where id=$1" [cid]
  if !company
    ret rep 404 {error:"kompaniya topilmadi"}
  rep 200 company
