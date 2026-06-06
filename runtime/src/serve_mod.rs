// Umumiy server/uzoq-ishlovchi boshqaruvi — bir jarayonda HTTP + WS + cron.run
// birga ishlashi uchun.
//
// Muammo: `http.serve`/`ws.serve` (va `cron.run`) ilgari har biri o'zicha ABADIY
// bloklardi (serve `block_on`, cron.run `loop { sleep }`). Faylда bir nechtasi
// chaqirilsa, birinchisi o'sha qatorda qotardi — qolganlari hech qachon ishga
// tushmasdi. REST + realtime + rejalashtirilgan vazifalarni (zamonaviy backend)
// bir jarayonda birlashtirib bo'lmasdi.
//
// Yechim: ular darhol bloklamaydi — `Interp.pending_servers` ro'yxatiga tavsif
// qo'shadi (deferred). Top-level kod tugagach `run_pending` hammasini BITTA umumiy
// tokio runtime'da spawn qilib bloklaydi. Hammasi bir runtime + bir `Interp`da
// bo'lgani uchun HTTP handler ichidan `ws.room.send` chaqirilsa WS ulanishlariga
// yetadi (shared state). `cron.run` esa shunchaki "dasturni ushlab tur" belgisi —
// scheduler allaqachon o'z fon thread'ida ishlaydi.

use std::sync::Arc;

use crate::interp::{Flow, Interp};

// Kutilayotgan deferred ish. `http.serve`/`ws.serve`/`cron.run` ro'yxatga qo'shadi.
#[derive(Clone, Copy)]
pub enum PendingServer {
    Http { port: u16 },
    Ws { port: u16 },
    // cron.run — port yo'q; scheduler fon thread'da ishlaydi, bu faqat dasturni
    // ushlab turish (top-level tugaganda chiqib ketmaslik) belgisi.
    Cron,
}

// Top-level kod tugagach chaqiriladi. Kutilayotgan ish bo'lsa, dasturni ushlab
// turadi (bloklaydi); aks holda darhol qaytadi — oddiy skript normal tugaydi.
//
// Tarmoq serveri (http/ws) bo'lsa: global bir marta muzlatiladi (lock-free
// qidiruv), bitta umumiy tokio runtime har serverni spawn qiladi va kutadi. Faqat
// `cron.run` bo'lsa (server yo'q): tokio runtime ham, freeze_globals ham KERAK
// EMAS — scheduler RwLock orqali o'qiydi (cron.on top-level o'rtasida bo'lishi
// mumkin, muzlatish keyingi global'larni yo'qotardi) — asosiy thread'ni uxlatib
// turamiz.
pub fn run_pending(interp: &Arc<Interp>) -> Result<(), Flow> {
    let pending = interp.pending_servers.lock().unwrap().clone();
    if pending.is_empty() {
        return Ok(());
    }

    // Tarmoq serverlari (cron'siz) — tokio runtime'da spawn qilinadi.
    let servers: Vec<PendingServer> = pending
        .iter()
        .copied()
        .filter(|p| !matches!(p, PendingServer::Cron))
        .collect();

    // Hech qanday tarmoq serveri yo'q, faqat cron.run — runtime'siz, freeze'siz
    // asosiy thread'ni uxlatib turamiz (scheduler fon thread'da davom etadi).
    if servers.is_empty() {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    // Global qidiruv lock-free bo'lsin (parallel handler'lar RwLock'ga urilmasin).
    // Bir marta — bu yerda hamma server boshlanishidan oldin. cron bo'lsa, u ham
    // shu frozen snapshot'dan o'qiydi (lookup ikkalasini qo'llaydi).
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
                // Cron yuqorida filtrlangan — bu yerga yetib kelmaydi.
                PendingServer::Cron => continue,
            };
            handles.push(h);
        }
        // Har bir serve_loop cheksiz (accept loop). Hammasini kutib turamiz —
        // amalda hech biri tugamaydi, jarayon shu yerda bloklanib qoladi.
        // (cron.run bo'lsa ham, server bloki dasturni ushlaydi — scheduler fonda.)
        for h in handles {
            let _ = h.await;
        }
    });
    Ok(())
}
