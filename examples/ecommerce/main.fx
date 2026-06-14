# main.fluxon — entry point
# Wires up all the modules and starts the server on port 8080.
# Each module registers its own http.on routes when it is loaded.
use http

# Schema (tbl) + endpoint modules.
use ./schema
use ./products
use ./cart
use ./checkout
use ./reviews
use ./aifeatures
use ./jobs

# Health check.
http.on :get "/health" \req -> rep 200 {status::ok}

# Register the cron jobs.
jobs.register_jobs

# Start the server.
log "Starting the e-commerce API on port 8080..."
http.serve 8080
