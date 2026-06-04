use db

# Poll database schema
tbl polls
  id       serial pk
  owner    str
  question str
  status   sym
  created  now

# Options for each poll
tbl options
  id      serial pk
  poll_id int ref:polls.id
  text    str
  votes   int
  created now

# Individual votes / responses
tbl responses
  id        serial pk
  poll_id   int ref:polls.id
  option_id int ref:options.id
  voter     str
  created   now
