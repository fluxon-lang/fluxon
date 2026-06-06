// Umumiy server boshqaruvi — bir jarayonda HTTP + WS birga ishlashi uchun.
//
// Muammo: `http.serve` va `ws.serve` ilgari har biri o'z tokio runtime'sini
// yaratib `block_on` bilan ABADIY bloklardi. Faylда ikkalasi chaqirilsa, birinchi
// `serve` o'sha qatorda qotardi — ikkinchisi hech qachon ishga tushmasdi. REST +
// realtime'ni (zamonaviy backend'ning eng keng tarqalgan naqshi) birlashtirib
// bo'lmasdi.
//
// Yechim: `http.serve`/`ws.serve` darhol bloklamaydi — `Interp.pending_servers`
// ro'yxatiga server tavsifini qo'shadi (deferred). Top-level kod tugagach
// `run_pending` BITTA umumiy tokio runtime yaratadi, har serverni unga `spawn`
// qiladi va abadiy bloklaydi. Hammasi bir runtime'da bo'lgani uchun HTTP handler
// ichidan `ws.room.send` chaqirilsa, WS ulanishlariga yetib boradi (shared state).

use std::sync::Arc;

use crate::interp::{Flow, Interp};

// Kutilayotgan server tavsifi. `http.serve`/`ws.serve` ro'yxatga qo'shadi.
#[derive(Clone, Copy)]
pub enum PendingServer {
    Http { port: u16 },
    Ws { port: u16 },
}

// Top-level kod tugagach chaqiriladi. Agar kutilayotgan server bo'lsa, global'ni
// bir marta muzlatadi, bitta umumiy multi-thread tokio runtime yaratib hamma
// serverni spawn qiladi va abadiy bloklaydi (serverlar to'xtamaydi). Server
// bo'lmasa darhol qaytadi — oddiy skript normal tugaydi.
pub fn run_pending(interp: &Arc<Interp>) -> Result<(), Flow> {
    let servers = interp.pending_servers.lock().unwrap().clone();
    if servers.is_empty() {
        return Ok(());
    }
    // Global qidiruv lock-free bo'lsin (parallel handler'lar RwLock'ga urilmasin).
    // Bir marta — bu yerda hamma server boshlanishidan oldin.
    interp.freeze_globals();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| Flow::err(format!("tokio runtime: {}", e)))?;

    rt.block_on(async move {
        let mut handles = Vec::new();
        for srv in servers {
            let interp = interp.clone();
            let h = match srv {
                PendingServer::Http { port } => {
                    tokio::spawn(async move { crate::http_mod::serve_loop(interp, port).await })
                }
                PendingServer::Ws { port } => {
                    tokio::spawn(async move { crate::ws_mod::serve_loop(interp, port).await })
                }
            };
            handles.push(h);
        }
        // Har bir serve_loop cheksiz (accept loop). Hammasini kutib turamiz —
        // amalda hech biri tugamaydi, jarayon shu yerda bloklanib qoladi.
        for h in handles {
            let _ = h.await;
        }
    });
    Ok(())
}
