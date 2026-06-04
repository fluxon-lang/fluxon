# Research — Flux qanday dizayn qilindi

Bu papka Flux tilining **dizayn jarayonini** saqlaydi. Til taxmin bilan emas,
AI modellarini stress-test qilish orqali qurildi.

## `language-design/`

### `round1-invented-langs/`
Uchta AI modelга (har xil model) "AI uchun til ixtiro qil" topshirig'i berildi.
Har biri o'z tilini ixtiro qilib, 3 ta loyiha (CLI, web API, realtime) yozdi.
**Natija:** mustaqil ravishda bir nechta model o'xshash g'oyalarga keldi
(symbol sintaksis, bitta loop, batteries) — "to'g'ri" dizayn bor.

### `round2-whatsapp/`
Xuddi shu, lekin real, murakkab loyiha bilan: WhatsApp-native AI biznes
yordamchisi. Til real yukda sinaldi. Opus versiyasi eng kam token + eng izchil
chiqdi → Flux shu asosga qurildi.

### `validation-tests/`
Flux spec'i tilni **hech ko'rmagan** modellarga (opus/sonnet/haiku) berilib,
real loyihalar yozdirildi. Har model topgan "spec bo'shliqlari" tilning haqiqiy
kamchiligini ko'rsatdi, keyin yopildi.

- (Round 1, URL qisqartiruvchi — kichik utility; 6 ta bo'shliq topildi: rand,
  str funksiyalar, redirect, ... Bu raund kodi interaktiv chiqqani uchun fayl
  sifatida saqlanmagan.)
- `round2-tickets/` — o'rta loyiha + AI. Chuqurroq bo'shliqlar (list metodlari,
  symbol↔DB, time, modul alias).
- `round3-large/` — katta loyihalar (e-commerce + chat). Eng chuqur teshiklar
  (tranzaksiya, websocket, map mutatsiya, lambda early-return).

Har raundda topilgan bo'shliqlar `docs/` dagi spec'ga qo'shildi.

> **Eslatma:** bu papkadagi kod — eksperiment artefaktlari, ishlab chiqarish
> kodi emas. Ba'zilarida ataylab xato bor (modellar topgan bo'shliqlar) —
> ular tilning qanday yaxshilanganini ko'rsatadi.
