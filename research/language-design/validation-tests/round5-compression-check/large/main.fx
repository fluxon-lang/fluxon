use http ws

# Import all modules
use ./companies as comp
use ./candidates as cand
use ./jobs as job_mod
use ./applications as app_mod
use ./realtime as rt
use ./cron_jobs as cron_mod

# ============================================================================
# HTTP Routes
# ============================================================================

# Companies
http.on :post "/companies" \req ->
  comp.create_company req

http.on :get "/companies" \req ->
  comp.list_companies req

http.on :get "/companies/:id" \req ->
  comp.get_company req

# Candidates
http.on :post "/candidates" \req ->
  cand.create_candidate req

http.on :get "/candidates" \req ->
  cand.list_candidates req

http.on :get "/candidates/:id" \req ->
  cand.get_candidate req

http.on :put "/candidates/:id" \req ->
  cand.update_candidate req

# Jobs
http.on :post "/companies/:company_id/jobs" \req ->
  job_mod.create_job req

http.on :get "/jobs" \req ->
  job_mod.list_jobs req

http.on :get "/jobs/:id" \req ->
  job_mod.get_job req

http.on :post "/jobs/:id/summarize" \req ->
  job_mod.summarize_job req

# Applications
http.on :post "/jobs/:job_id/apply" \req ->
  app_mod.apply_to_job req

http.on :get "/candidates/:candidate_id/applications" \req ->
  app_mod.get_candidate_applications req

http.on :get "/jobs/:job_id/applications" \req ->
  app_mod.get_job_applications req

http.on :get "/applications/:id" \req ->
  app_mod.get_application req

http.on :put "/applications/:id/status" \req ->
  app_mod.update_application_status req

# ============================================================================
# WebSocket Setup
# ============================================================================

rt.setup_websocket

# ============================================================================
# Cron Jobs
# ============================================================================

cron_mod.setup_daily_cron

# ============================================================================
# Server Start
# ============================================================================

log "Starting Job Board Platform..."
http.serve 8080
ws.serve 9000
log "HTTP on 8080, WebSocket on 9000"
