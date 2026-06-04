# User management

use db

# Create a new user
exp fn create_user username email
  existing = db.one "select id from users where email = $1" [email]
  if existing
    fail "Email already exists"

  user = db.ins "users" {
    username: username
    email: email
    status: :online
  }
  ret user

# Get user by id
exp fn get_user id
  ret db.one "select * from users where id = $1" [id]

# Get user by username
exp fn get_user_by_username username
  ret db.one "select * from users where username = $1" [username]

# List all users
exp fn list_users
  ret db.q "select id, username, email, status from users order by username"

# Update user status
exp fn set_user_status user_id status
  db.up "users" {status: status} {id: user_id}
  ret db.one "select * from users where id = $1" [user_id]

# Get user presence (online status across channels)
exp fn get_user_presence user_id
  channels = db.q "select distinct c.id, c.name from channels c join memberships m on c.id = m.channel where m.user = $1" [user_id]
  ret {
    user_id: user_id
    channels: channels
    status: :online
  }
