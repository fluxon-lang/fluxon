# Main entry point - Customer Support Ticket System API

use http log
use ./tickets_handler
use ./replies_handler
use ./cron_jobs

# Initialize database schemas (assumes tables exist or auto-created)
# In production, run schema.flux separately

# Wire up HTTP endpoints
http.on :post "/tickets" \req -> create_ticket req
http.on :get "/tickets" \req -> list_tickets req
http.on :get "/tickets/:id" \req -> get_ticket req
http.on :post "/tickets/:id/reply" \req -> post_reply req

# Initialize cron jobs
schedule_cron_jobs

# Start server
log "Starting support ticket API on port 8080..."
http.serve 8080
