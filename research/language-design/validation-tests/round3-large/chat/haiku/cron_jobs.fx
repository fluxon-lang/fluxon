# Scheduled background jobs

use cron
use db
use time
use log
use ./ai_service as ai_mod

# Hourly stats job: log active channels and message volume
fn log_hourly_stats
  # Get message volume in last hour
  msg_count_result = db.one "
    select count(*) as cnt from messages where created > $1
  " [time.ago 1 :hr]
  msg_count = msg_count_result.cnt ?? 0

  # Get active channels (channels with messages in last hour)
  active_channels = db.q "
    select c.id, c.name, count(*) as msg_count
    from messages m
    join channels c on m.channel = c.id
    where m.created > $1
    group by c.id, c.name
    order by msg_count desc
    limit 10
  " [time.ago 1 :hr]

  # Get unique active users
  active_users_result = db.one "
    select count(distinct user) as cnt from messages where created > $1
  " [time.ago 1 :hr]
  active_users = active_users_result.cnt ?? 0

  # Log stats
  log "STATS: ${msg_count} messages, ${active_users} users, ${active_channels.len} active channels"

  each ch in active_channels
    log "  - ${ch.name}: ${ch.msg_count} msgs"

  ret {
    timestamp: time.now
    message_count: msg_count
    user_count: active_users
    active_channels: active_channels.len
  }

# Register hourly stats job
cron.hr 0 log_hourly_stats

# Daily user activity summary
fn daily_user_activity
  yesterday = time.ago 24 :hr

  most_active = db.q "
    select u.id, u.username, count(*) as msg_count
    from messages m
    join users u on m.user = u.id
    where m.created > $1
    group by u.id, u.username
    order by msg_count desc
    limit 10
  " [yesterday]

  log "Daily top posters:"
  each user in most_active
    log "  ${user.username}: ${user.msg_count} messages"

  ret most_active

cron.dy 2 0 daily_user_activity

# Cleanup spam: detect and log suspicious user patterns
fn cleanup_spam_patterns
  # Find users with high message volume in past hour
  suspicious = db.q "
    select u.id, u.username, count(*) as msg_count
    from messages m
    join users u on m.user = u.id
    where m.created > $1
    group by u.id, u.username
    having count(*) > 100
    order by msg_count desc
  " [time.ago 1 :hr]

  log "Spam detection: found ${suspicious.len} suspicious users"

  each user in suspicious
    spam_check = ai_mod.detect_spam_user user.id 1
    if spam_check.is_likely_spam
      log "SPAM ALERT: ${user.username} (${spam_check.message_count} msgs in ${spam_check.channel_count} channels)"

  ret suspicious

cron.hr 15 cleanup_spam_patterns

# Inactive user cleanup (mark offline after 30 min inactivity)
fn mark_inactive_users
  thirty_min_ago = time.ago 30 :min

  # Find users with no activity in 30 min
  inactive = db.q "
    select distinct u.id, u.username
    from users u
    where u.status = :online
    and u.id not in (
      select distinct user from messages where created > $1
    )
  " [thirty_min_ago]

  log "Marking ${inactive.len} users as offline"

  each user in inactive
    db.up "users" {status: :offline} {id: user.id}

  ret inactive

cron.hr 30 mark_inactive_users

# Channel archival: archive channels with no messages in 90 days
fn archive_inactive_channels
  ninety_days_ago = time.ago 90 :day

  inactive_channels = db.q "
    select c.id, c.name
    from channels c
    where c.id not in (
      select distinct channel from messages where created > $1
    )
    and c.is_private = false
  " [ninety_days_ago]

  log "Found ${inactive_channels.len} channels inactive for 90 days"

  # In real app, move to archive table or mark archived
  # For now, just log
  each ch in inactive_channels
    log "  - Archive candidate: ${ch.name}"

  ret inactive_channels

cron.dy 3 0 archive_inactive_channels

log "Cron jobs registered"
