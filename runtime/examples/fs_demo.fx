# fs battery demo — local filesystem primitives.
# Run: cargo run -- run examples/fs_demo.fx
#
# Non-blocking (not a server) — also usable as a smoke-test. Works in a temporary
# folder and cleans up after itself at the end.

dir = "/tmp/fluxon_fs_demo"

# Prepare the folder (idempotent — not an error if it already exists).
fs.mkdirp dir
log "folder ready:" dir

# Write a config (with json.enc) and read it back.
conf = "${dir}/conf.json"
fs.write conf (json.enc {port:8080 name:"fluxon"})
cfg = json.dec (fs.read conf)
log "read port:" cfg.port

# Append to a log file sequentially.
audit = "${dir}/audit.log"
fs.append audit "started\n"
fs.append audit "finished\n"
log "audit contents:" (fs.read audit)

# Reading a missing file — nil (not an error).
missing = fs.read "${dir}/missing.txt"
log "missing file:" missing

# List the folder contents.
log "files:" (fs.ls dir)

# Cleanup.
fs.del conf
fs.del audit
fs.del dir
log "cleaned up, folder exists:" (fs.exists dir)
