# users.flux — user management: create users, lookup, presence helpers.
use db

# Create a new user. Defaults status to :offline until they connect.
exp fn create_user username email
  if !username
    fail "username required"
  if !email
    fail "email required"
  existing = db.one "select * from users where username=$1" [username]
  if existing
    fail "username taken"
  ret db.ins "users" {username:username email:email status::offline}

# Fetch a user by id (nil if missing).
exp fn get_user id
  ret db.one "select * from users where id=$1" [id]

# Fetch a user by username.
exp fn by_username username
  ret db.one "select * from users where username=$1" [username]

# Update a user's presence/status symbol (:online :offline :away).
exp fn set_status id status
  db.up "users" {status:status} {id:id}
  ret get_user id

# Authenticate a user for a websocket/session by username.
# Lightweight: in a real system this checks a token; here we resolve the
# username to a user row and fail if not found.
exp fn authenticate username
  u = by_username username
  u ?? (fail "auth failed: unknown user $username")
  ret u
