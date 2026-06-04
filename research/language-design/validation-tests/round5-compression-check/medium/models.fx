use db

# Validate and create a new poll
exp fn create_poll owner question options_list
  if str.len question == 0
    fail 400 "question kerak"
  if options_list.len < 2
    fail 400 "kamida 2 ta variant kerak"

  each opt in options_list
    if str.len opt == 0
      fail 400 "barcha variant'lar bo'sh bo'lmasa kerak"

  poll = db.tx \->
    p = db.ins "polls" {owner:owner question:question status::open}
    each opt_text in options_list
      db.ins "options" {poll_id:p.id text:opt_text votes:0}
    ret p

  ret poll

# Get a poll with all options and vote counts
exp fn get_poll poll_id
  poll = db.one "select * from polls where id=$1" [poll_id]!
  if !poll
    fail 404 "poll topilmadi"

  options = db.q "select * from options where poll_id=$1 order by id" [poll_id]

  ret {
    id:poll.id
    owner:poll.owner
    question:poll.question
    status:poll.status
    created:poll.created
    options:options
    total_votes:(db.one "select sum(votes) v from options where poll_id=$1" [poll_id]).v ?? 0
  }

# Cast a vote (validates poll is open and option belongs to poll)
exp fn cast_vote poll_id option_id voter
  poll = db.one "select * from polls where id=$1" [poll_id]!
  if !poll
    fail 404 "poll topilmadi"

  if poll.status != :open
    fail 422 "poll yopiq"

  option = db.one "select * from options where id=$1 and poll_id=$2" [option_id poll_id]!
  if !option
    fail 400 "variant topilmadi yoki bu poll'ga tegishli emas"

  db.tx \->
    db.ins "responses" {poll_id:poll_id option_id:option_id voter:voter}
    db.up "options" {votes:option.votes + 1} {id:option_id}

  ret {success:true option_id:option_id}

# Close a poll
exp fn close_poll poll_id
  poll = db.one "select * from polls where id=$1" [poll_id]!
  if !poll
    fail 404 "poll topilmadi"

  db.up "polls" {status::closed} {id:poll_id}

  ret {success:true status::closed}

# List polls with optional status filter
exp fn list_polls filter_status
  if filter_status
    polls = db.q "select * from polls where status=$1 order by created desc" [filter_status]
  else
    polls = db.q "select * from polls order by created desc"

  each p in polls
    p.total_votes <- db.one "select sum(votes) v from options where poll_id=$1" [p.id]
    p.total_votes <- p.total_votes.v ?? 0

  ret polls

# Get poll results for summarization
exp fn get_poll_results poll_id
  poll = db.one "select * from polls where id=$1" [poll_id]!
  if !poll
    fail 404 "poll topilmadi"

  options = db.q "select * from options where poll_id=$1 order by votes desc" [poll_id]

  ret {
    question:poll.question
    status:poll.status
    options:options
  }
