use http

# Import schema to ensure tables are created
use ./schema

# Import API endpoints
use ./api

# Import cron jobs
use ./cron_tasks

# Start HTTP server on port 8080
http.serve 8080
