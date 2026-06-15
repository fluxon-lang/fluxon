// Fluxon HTTP battery — server (http.on/http.serve/rep) and client (http.get/post).
//
// The server is built on tokio + hyper. Because Fluxon handlers are synchronous
// tree-walking, each request runs inside `spawn_blocking` — this makes the CPU
// work TRULY PARALLEL without blocking tokio workers (Value: Send+Sync, the
// thread-safety refactor guarantees this).
//
// `rep status body` -> {__resp:true status body} map (builtins.rs::install).
// `fail status "msg"` -> Flow::Fail -> JSON error response.

mod client;
mod interp;
mod limits;
mod middleware;
mod request;
mod response;
mod routing;
mod server;
mod static_files;

// Re-exports preserving the public surface used by other modules.
// Interp stores Vec<Route>/Vec<Middleware>/Option<CorsConfig>/Vec<StaticMount>;
// serve_mod calls bind/serve_loop; ai_mod reuses the client runtime/pool.
pub(crate) use client::{client_runtime, pooled_http_client};
pub use middleware::{CorsConfig, Middleware};
pub use routing::Route;
pub use server::{bind, serve_loop};
pub use static_files::StaticMount;
