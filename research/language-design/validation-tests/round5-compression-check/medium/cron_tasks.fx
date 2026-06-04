use db log time

# Hourly cron job: log active polls and total votes
fn log_hourly_stats hr min
  active_polls = db.q "select id, question, owner from polls where status=$1" [:open]

  if active_polls.len > 0
    total_votes <- 0
    each poll in active_polls
      poll_votes = db.one "select sum(votes) v from options where poll_id=$1" [poll.id]
      total_votes <- total_votes + (poll_votes.v ?? 0)

    log "HOURLY STATS: ${active_polls.len} active polls, ${total_votes} total votes at ${time.now}"
  else
    log "HOURLY STATS: no active polls"

# Register hourly cron job (runs every hour at :30 minutes)
cron.hr 30 log_hourly_stats
