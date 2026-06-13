// Shared server / long-running control — so HTTP + WS + cron.run can run together
// in a single process.
//
// Problem: `http.serve`/`ws.serve` (and `cron.run`) each used to block FOREVER on
// their own (serve via `block_on`, cron.run via `loop { sleep }`). If several were
// called in one file, the first one would hang on that line — the rest never
// started. REST + realtime + scheduled tasks (a modern backend) could not be
// combined in a single process.
//
// Solution: they no longer block immediately — they append a descriptor to the
// `Interp.pending_servers` list (deferred). Once top-level code finishes,
// `run_pending` spawns all of them on ONE shared tokio runtime and blocks. Since
// everything lives on a single runtime + single `Interp`, calling `ws.room.send`
// from inside an HTTP handler reaches the WS connections (shared state). `cron.run`
// is merely a "keep the program alive" marker — the scheduler already runs in its
// own background thread.

use std::sync::Arc;

use crate::interp::{Flow, Interp};

// A pending deferred job. `http.serve`/`ws.serve`/`cron.run` append to the list.
#[derive(Clone, Copy)]
pub enum PendingServer {
    // max_body — request body size limit (bytes); 0 = unlimited (#91).
    Http { port: u16, max_body: usize },
    Ws { port: u16 },
    // cron.run — no port; the scheduler runs in a background thread, this is just
    // a marker to keep the program alive (don't exit when top-level finishes).
    Cron,
}

// Called once top-level code finishes. If there is pending work, it keeps the
// program alive (blocks); otherwise it returns immediately — a plain script ends
// normally.
//
// If there is a network server (http/ws): globals are frozen once (lock-free
// lookup), a single shared tokio runtime spawns and awaits each server. If there
// is only `cron.run` (no server): neither a tokio runtime nor freeze_globals is
// needed — the scheduler reads via RwLock (cron.on may appear in the middle of
// top-level, and freezing would lose the later globals) — so we just put the main
// thread to sleep.
pub fn run_pending(interp: &Arc<Interp>) -> Result<(), Flow> {
    let pending = interp.pending_servers.lock().unwrap().clone();
    if pending.is_empty() {
        return Ok(());
    }

    // Network servers (excluding cron) — spawned on the tokio runtime.
    let servers: Vec<PendingServer> = pending
        .iter()
        .copied()
        .filter(|p| !matches!(p, PendingServer::Cron))
        .collect();

    // No network server at all, only cron.run — put the main thread to sleep
    // without a runtime or freeze (the scheduler continues in its background thread).
    if servers.is_empty() {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    // Make global lookup lock-free (so parallel handlers don't contend on the
    // RwLock). Done once — here, before any server starts. If cron is present, it
    // too reads from this frozen snapshot (lookup supports both).
    interp.freeze_globals();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| Flow::err(format!("tokio runtime: {}", e)))?;

    // Bind the ports FIRST (before spawning the accept loop). If a port is in use,
    // bind returns an error → the whole `run_pending` exits with `Err` → the
    // process exit code is != 0 (issue #108: don't fool deploy/supervisor with a
    // silent success). Because binding happens upfront the error surfaces
    // deterministically — accept loops run forever and never finish, so we cannot
    // rely on their `await` for this.
    rt.block_on(async move {
        let mut handles = Vec::new();
        for srv in servers {
            let interp = interp.clone();
            match srv {
                PendingServer::Http { port, max_body } => {
                    let listener = crate::http_mod::bind(port).await?;
                    handles.push(tokio::spawn(async move {
                        crate::http_mod::serve_loop(interp, listener, max_body).await
                    }));
                }
                PendingServer::Ws { port } => {
                    let listener = crate::ws_mod::bind(port).await?;
                    handles.push(tokio::spawn(async move {
                        crate::ws_mod::serve_loop(interp, listener).await
                    }));
                }
                // Cron was filtered out above — never reaches here.
                PendingServer::Cron => continue,
            }
        }
        // Each serve_loop runs forever (accept loop). We await all of them — in
        // practice none ever finish, so the process blocks here.
        // (Even with cron.run, the server block keeps the program alive — scheduler
        // runs in the background.)
        for h in handles {
            let _ = h.await;
        }
        Ok::<(), Flow>(())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_port_run_pending_err() {
        // A bind error propagates from `run_pending` as `Err(Flow::Error)`
        // (issue #108) — not a silent `Ok(())`. In interp.rs this turns into an
        // exit code != 0 (so deploy/supervisor notices the error). We occupy the
        // port with a std listener, then set up a pending Http server on that port.
        let occupied = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let port = occupied.local_addr().unwrap().port();

        let interp = crate::interp::Interp::new_arc();
        interp
            .pending_servers
            .lock()
            .unwrap()
            .push(PendingServer::Http { port, max_body: 0 });

        let res = run_pending(&interp);
        assert!(
            matches!(res, Err(Flow::Error(_))),
            "occupied port → expected Err(Flow::Error), not a silent Ok"
        );
    }
}
