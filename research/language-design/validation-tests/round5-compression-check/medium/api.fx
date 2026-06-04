use http
use ./models
use ./ai_summary

# POST /polls — create a poll with question and options
http.on :post "/polls" \req ->
  if !req.body.owner
    ret rep 400 {error:"owner kerak"}
  if !req.body.question
    ret rep 400 {error:"question kerak"}
  if !req.body.options
    ret rep 400 {error:"options list kerak"}

  if req.body.options.len == 0
    ret rep 400 {error:"kamida 2 ta variant kerak"}

  poll = models.create_poll req.body.owner req.body.question req.body.options

  ret rep 201 {
    id:poll.id
    owner:poll.owner
    question:poll.question
    status:poll.status
    created:poll.created
  }

# GET /polls/:id — get poll with options and vote counts
http.on :get "/polls/:id" \req ->
  poll_id = str.int req.params.id

  if !poll_id
    ret rep 400 {error:"invalid poll id"}

  poll = models.get_poll poll_id

  ret rep 200 poll

# GET /polls — list polls with optional status filter
http.on :get "/polls" \req ->
  filter_status = nil
  if req.query.status
    match req.query.status
      "open" -> filter_status <- :open
      "closed" -> filter_status <- :closed
      _ -> filter_status <- nil

  polls = models.list_polls filter_status

  ret rep 200 {polls:polls}

# POST /polls/:id/vote — cast a vote
http.on :post "/polls/:id/vote" \req ->
  poll_id = str.int req.params.id

  if !poll_id
    ret rep 400 {error:"invalid poll id"}

  if !req.body.option_id
    ret rep 400 {error:"option_id kerak"}

  if !req.body.voter
    ret rep 400 {error:"voter kerak"}

  result = models.cast_vote poll_id req.body.option_id req.body.voter

  ret rep 200 result

# POST /polls/:id/close — close a poll
http.on :post "/polls/:id/close" \req ->
  poll_id = str.int req.params.id

  if !poll_id
    ret rep 400 {error:"invalid poll id"}

  result = models.close_poll poll_id

  ret rep 200 result

# POST /polls/:id/summarize — AI summary of results
http.on :post "/polls/:id/summarize" \req ->
  poll_id = str.int req.params.id

  if !poll_id
    ret rep 400 {error:"invalid poll id"}

  summary = ai_summary.summarize_poll poll_id

  ret rep 200 summary
