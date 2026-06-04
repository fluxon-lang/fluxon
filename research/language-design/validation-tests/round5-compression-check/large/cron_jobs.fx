use db log time json

# Daily reporting job - runs at 9:00 AM every day
exp fn setup_daily_cron
  cron.dy 9 0 \day hour minute ->
    log "=== Daily Job Board Report ==="

    # Count open jobs
    open_jobs_result = db.one "select count(*) c from jobs where status=$1" [:open]
    open_jobs = open_jobs_result.c ?? 0

    # Count applications created today
    today_start = time.ago 24 :hr
    today_apps_result = db.one "select count(*) c from applications where created > $1" [today_start]
    today_apps = today_apps_result.c ?? 0

    # Calculate shortlist rate
    total_apps_result = db.one "select count(*) c from applications" []
    total_apps = total_apps_result.c ?? 0

    shortlist_result = db.one "select count(*) c from applications where status=$1" [:shortlisted]
    shortlist_count = shortlist_result.c ?? 0

    shortlist_rate = 0.0
    if total_apps > 0
      shortlist_rate = (shortlist_count * 100.0) / total_apps

    report = {
      timestamp:(time.now)
      open_jobs:open_jobs
      applications_today:today_apps
      total_applications:total_apps
      shortlisted_count:shortlist_count
      shortlist_rate_percent:shortlist_rate
    }

    log "Report: ${json.enc report}"

    # You could also store this in a reports table:
    # db.ins "job_reports" report
