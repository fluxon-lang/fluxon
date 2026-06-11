# main.fluxon — entry point: wire up routes and start the server

use http
use ./tickets
use ./cron

# Register ticket routes
http.on :post "/tickets"           \req -> tickets.handle_create_ticket req
http.on :get  "/tickets"           \req -> tickets.handle_list_tickets req
http.on :get  "/tickets/:id"       \req -> tickets.handle_get_ticket req
http.on :post "/tickets/:id/reply" \req -> tickets.handle_add_reply req

# Register the daily cron job
cron.register_cron

# Start the HTTP server
http.serve 8080
