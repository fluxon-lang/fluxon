# Til nomi: **Petal**

## Asosiy g'oya

Petal — deklarativ, reaktiv UI tili bo'lib, "hamma narsa gulday ochiladi" falsafasi asosida qurilgan. Asosiy g'oya: UI — bu **ma'lumotning vizual proeksiyasi**, shuning uchun state va ko'rinish bitta joyda, bitta sintaksda yashaydi. Petal'da kod chapdan o'ngga emas, **daraxt kabi** ochiladi — har bir komponent `bloom` kalit so'zi bilan boshlanadi va ichida state, style, va render bir-biriga o'ralgan holda yashaydi.

Petal ikki fundamental g'oyani birlashtiradi: **CSS-in-tree** (stillar komponent ichida, lekin kaskad yo'q — har komponent izolyatsiyalangan) va **reactive binding** (`~` belgisi bilan e'lon qilingan har qanday o'zgaruvchi avtomatik reaktiv). Backend bilan muloqot `fetch` o'rniga `bloom`ning `source` atributi orqali — bu tilni ishlatan kompilyator/runtime HTTP so'rovlarni boshqaradi. Event'lar `on:click`, `on:input` kabi, lekin ular to'g'ridan-to'g'ri state mutatsiyasiga bog'langan.

## Sintaksis qoidalari

```
// Komponent e'loni
bloom <KomponentNomi> {
  // Reaktiv state (~)
  ~state_nomi: <tip> = <boshlangich_qiymat>

  // Computed (=>) avtomatik qayta hisoblanadi
  ~computed_qiymat => state_nomi * 2

  // Ma'lumot manbai (API)
  source products from "/api/products" refresh:30s

  // Stil bloki (CSS-like, lekin scope'd)
  style {
    root: {
      bg: #1a1a2e
      pad: 24px
      radius: 12px
    }
    .title: {
      font: "Cormorant Garamond" 28px bold
      color: #e8c4a0
    }
  }

  // Render bloki — daraxt sintaksisi
  render {
    box.root {
      text.title { "Sarlavha" }

      // Loop
      each item in items {
        box.card { text { item.name } }
      }

      // Shart
      if ~isLoading {
        spinner { }
      } else {
        list { ... }
      }

      // Event binding
      button on:click { ~count += 1 } { "Bosing" }

      // Input binding (ikki tomonlama)
      input bind:~searchQuery placeholder:"Qidiring..."

      // Child komponent chaqirish
      <ProductCard item=item on:delete={ ~items.remove(item) } />
    }
  }
}

// Tipdagi o'zgaruvchilar
// ~name: str = "Gullar do'koni"
// ~count: int = 0
// ~price: float = 12.50
// ~active: bool = true
// ~list: arr[Product] = []

// API so'rov (source)
// source products from "/api/products"    // GET, bir martalik
// source orders from "/api/orders" refresh:60s   // har 60 soniyada yangilash
// source POST "/api/products" as createProduct  // mutation

// Navigatsiya
// route "/products" -> <ProductsPage />
// route "/orders"   -> <OrdersPage />
```

## To'liq dashboard kodi

```petal
// ============================================================
//  PETAL v1.0 — Flower Shop Admin Dashboard
//  Fayl: dashboard.petal
//  Entry point: <App />
// ============================================================

// ─── GLOBAL THEME ───────────────────────────────────────────
theme FlowerDark {
  --bg-deep:       #0d0d14
  --bg-card:       #16161f
  --bg-hover:      #1e1e2e
  --border:        #2a2a3d
  --accent-gold:   #d4a853
  --accent-rose:   #c45c7a
  --accent-sage:   #6a9e7f
  --accent-lavender: #8b7fc7
  --text-primary:  #f0ece4
  --text-secondary: #8a8a9e
  --text-muted:    #4a4a5e
  --danger:        #e05c5c
  --success:       #5cae82
  --warning:       #e0aa5c

  font-display: "Cormorant Garamond"
  font-body:    "DM Sans"
  font-mono:    "JetBrains Mono"
}


// ─── SHARED TYPES ───────────────────────────────────────────
type Product {
  id: int
  name: str
  category: str
  price: float
  stock: int
  image: str
  sold: int
}

type Order {
  id: int
  customer: str
  phone: str
  items: arr[OrderItem]
  total: float
  status: enum["pending","processing","delivered","cancelled"]
  created_at: datetime
  address: str
}

type OrderItem {
  product_name: str
  qty: int
  price: float
}

type Customer {
  id: int
  name: str
  email: str
  phone: str
  total_orders: int
  total_spent: float
  last_order: datetime
  favorite_flower: str
}

type DailyStat {
  hour: str
  revenue: float
  orders: int
}


// ─── MOCK DATA STORE ────────────────────────────────────────
// (Runtime da bu tashqi API dan keladi; demo uchun inline)
store AppData {
  products: arr[Product] = [
    { id:1, name:"Qizil atirgul",   category:"Atirgul",   price:12000, stock:85,  image:"🌹", sold:342 },
    { id:2, name:"Oq lola",         category:"Lola",      price:8500,  stock:120, image:"🌷", sold:218 },
    { id:3, name:"Sariq chamanzor", category:"Dala guli", price:6000,  stock:60,  image:"🌼", sold:175 },
    { id:4, name:"Binafsha orkide", category:"Orkide",    price:45000, stock:22,  image:"🌸", sold:89  },
    { id:5, name:"Nilufar",         category:"Suv guli",  price:15000, stock:45,  image:"💐", sold:134 },
    { id:6, name:"Lavanda dastasi", category:"Lavanda",   price:22000, stock:38,  image:"💜", sold:201 },
    { id:7, name:"Qizg'aldoq",      category:"Dala guli", price:4500,  stock:200, image:"🌺", sold:95  },
    { id:8, name:"Pion",            category:"Peony",     price:18000, stock:33,  image:"🌸", sold:156 }
  ]

  orders: arr[Order] = [
    { id:1001, customer:"Malika Yusupova",   phone:"+998901234567", items:[{product_name:"Qizil atirgul",qty:11,price:12000},{product_name:"Lavanda dastasi",qty:1,price:22000}], total:154000, status:"delivered",   created_at:"2026-06-07 09:14", address:"Chilonzor 5-kv, 12-uy" },
    { id:1002, customer:"Bobur Toshmatov",   phone:"+998907654321", items:[{product_name:"Oq lola",qty:7,price:8500}],                                                             total:59500,  status:"processing",  created_at:"2026-06-07 10:32", address:"Yunusobod 11-kv, 34-uy" },
    { id:1003, customer:"Dilnoza Rahimova",  phone:"+998911112233", items:[{product_name:"Binafsha orkide",qty:1,price:45000}],                                                    total:45000,  status:"pending",     created_at:"2026-06-07 11:05", address:"Mirzo Ulugbek, 7-uy" },
    { id:1004, customer:"Jasur Karimov",     phone:"+998934445566", items:[{product_name:"Pion",qty:5,price:18000},{product_name:"Sariq chamanzor",qty:3,price:6000}],             total:108000, status:"pending",     created_at:"2026-06-07 11:48", address:"Shayxontohur, 22-uy" },
    { id:1005, customer:"Sevara Nazarova",   phone:"+998956667788", items:[{product_name:"Nilufar",qty:1,price:15000}],                                                            total:15000,  status:"cancelled",   created_at:"2026-06-07 08:20", address:"Sergeli, 9-kv" },
    { id:1006, customer:"Alisher Xoliqov",   phone:"+998978889900", items:[{product_name:"Qizil atirgul",qty:21,price:12000}],                                                     total:252000, status:"delivered",   created_at:"2026-06-06 16:30", address:"Bektemir, 3-uy" },
    { id:1007, customer:"Nargiza Sobirov",   phone:"+998912223344", items:[{product_name:"Lavanda dastasi",qty:2,price:22000},{product_name:"Oq lola",qty:5,price:8500}],          total:86500,  status:"processing",  created_at:"2026-06-07 12:15", address:"Olmazor, 15-kv" }
  ]

  customers: arr[Customer] = [
    { id:1, name:"Malika Yusupova",  email:"malika@mail.uz",   phone:"+998901234567", total_orders:12, total_spent:980000,  last_order:"2026-06-07", favorite_flower:"Qizil atirgul" },
    { id:2, name:"Bobur Toshmatov",  email:"bobur@gmail.com",  phone:"+998907654321", total_orders:7,  total_spent:412000,  last_order:"2026-06-07", favorite_flower:"Oq lola" },
    { id:3, name:"Dilnoza Rahimova", email:"dilnoza@yahoo.com",phone:"+998911112233", total_orders:3,  total_spent:185000,  last_order:"2026-06-07", favorite_flower:"Orkide" },
    { id:4, name:"Jasur Karimov",    email:"jasur@mail.uz",    phone:"+998934445566", total_orders:18, total_spent:1540000, last_order:"2026-06-07", favorite_flower:"Pion" },
    { id:5, name:"Sevara Nazarova",  email:"sevara@gmail.com", phone:"+998956667788", total_orders:5,  total_spent:225000,  last_order:"2026-06-07", favorite_flower:"Nilufar" },
    { id:6, name:"Alisher Xoliqov",  email:"alisher@mail.uz",  phone:"+998978889900", total_orders:24, total_spent:3200000, last_order:"2026-06-06", favorite_flower:"Qizil atirgul" }
  ]

  dailyStats: arr[DailyStat] = [
    { hour:"08:00", revenue:45000,  orders:3  },
    { hour:"09:00", revenue:88000,  orders:7  },
    { hour:"10:00", revenue:124000, orders:9  },
    { hour:"11:00", revenue:210000, orders:15 },
    { hour:"12:00", revenue:175000, orders:12 },
    { hour:"13:00", revenue:95000,  orders:8  },
    { hour:"14:00", revenue:140000, orders:11 },
    { hour:"15:00", revenue:190000, orders:14 },
    { hour:"16:00", revenue:88000,  orders:6  },
    { hour:"17:00", revenue:52000,  orders:4  }
  ]

  settings: obj = {
    shopName: "Gullar Bog'i",
    ownerName: "Fotima Ergasheva",
    phone: "+998 90 123 45 67",
    address: "Toshkent sh., Chilonzor tumani, Navruz ko'chasi 14",
    openTime: "08:00",
    closeTime: "20:00",
    workDays: ["Du","Se","Ch","Pa","Ju","Sh"],
    primaryColor: "#d4a853",
    accentColor: "#c45c7a",
    currency: "UZS",
    taxRate: 12
  }
}


// ─── APP ROOT ───────────────────────────────────────────────
bloom App {
  ~activePage: str = "dashboard"
  ~sidebarOpen: bool = true
  ~notification: str = ""
  ~notifType: str = "success"

  style {
    root: {
      display: grid
      grid: "sidebar main" / 260px 1fr
      height: 100vh
      overflow: hidden
      bg: var(--bg-deep)
      font: var(--font-body)
      color: var(--text-primary)
      transition: grid-template-columns 0.3s ease
    }
    root.collapsed: {
      grid: "sidebar main" / 72px 1fr
    }
    .notif: {
      position: fixed
      top: 24px
      right: 24px
      z-index: 9999
      pad: 14px 22px
      radius: 10px
      font: var(--font-body) 14px
      shadow: 0 8px 32px rgba(0,0,0,0.4)
      animate: slideInRight 0.3s ease
    }
    .notif.success: { bg: var(--success);  color: #fff }
    .notif.error:   { bg: var(--danger);   color: #fff }
    .notif.info:    { bg: var(--accent-lavender); color: #fff }
  }

  render {
    box.root class:{ sidebarOpen ? "" : "collapsed" } {
      <Sidebar
        active=~activePage
        collapsed=~sidebarOpen.not
        on:navigate={ ~activePage = $event }
        on:toggle={ ~sidebarOpen = ~sidebarOpen.not }
      />

      box style:"grid-area:main; overflow:auto;" {
        if ~activePage == "dashboard"  { <DashboardPage  on:notify={~notification=$event.msg; ~notifType=$event.type} /> }
        if ~activePage == "products"   { <ProductsPage   on:notify={~notification=$event.msg; ~notifType=$event.type} /> }
        if ~activePage == "orders"     { <OrdersPage     on:notify={~notification=$event.msg; ~notifType=$event.type} /> }
        if ~activePage == "customers"  { <CustomersPage  /> }
        if ~activePage == "settings"   { <SettingsPage   on:notify={~notification=$event.msg; ~notifType=$event.type} /> }
      }

      if ~notification != "" {
        box.notif class:~notifType {
          text { ~notification }
        }
        // Auto-dismiss after 3s
        timer 3000 { ~notification = "" }
      }
    }
  }
}


// ─── SIDEBAR ────────────────────────────────────────────────
bloom Sidebar {
  props {
    active: str
    collapsed: bool
  }

  ~navItems: arr = [
    { id:"dashboard", icon:"⬡",  label:"Bosh sahifa"  },
    { id:"products",  icon:"✿",  label:"Mahsulotlar"  },
    { id:"orders",    icon:"◈",  label:"Buyurtmalar"  },
    { id:"customers", icon:"◉",  label:"Mijozlar"     },
    { id:"settings",  icon:"⚙",  label:"Sozlamalar"   }
  ]

  style {
    root: {
      grid-area: sidebar
      bg: var(--bg-card)
      border-right: 1px solid var(--border)
      display: flex
      flex-direction: column
      height: 100vh
      overflow: hidden
      transition: all 0.3s ease
      position: relative
    }
    .logo-zone: {
      pad: 28px 24px 20px
      border-bottom: 1px solid var(--border)
      display: flex
      align-items: center
      gap: 12px
    }
    .logo-icon: {
      font-size: 28px
      flex-shrink: 0
      animate: spin 8s linear infinite
    }
    .logo-text: {
      font: var(--font-display) 20px 600
      color: var(--accent-gold)
      white-space: nowrap
      overflow: hidden
      opacity: 1
      transition: opacity 0.2s
    }
    .logo-text.hidden: { opacity: 0; width: 0 }
    .nav-section: {
      flex: 1
      pad: 16px 0
      overflow-y: auto
    }
    .nav-item: {
      display: flex
      align-items: center
      gap: 14px
      pad: 13px 24px
      cursor: pointer
      transition: all 0.2s ease
      border-left: 3px solid transparent
      color: var(--text-secondary)
      font: var(--font-body) 14px 500
      white-space: nowrap
    }
    .nav-item:hover: {
      bg: var(--bg-hover)
      color: var(--text-primary)
      border-left-color: var(--accent-gold)
    }
    .nav-item.active: {
      bg: linear-gradient(90deg, rgba(212,168,83,0.12) 0%, transparent 100%)
      color: var(--accent-gold)
      border-left-color: var(--accent-gold)
    }
    .nav-icon: {
      font-size: 18px
      flex-shrink: 0
      width: 22px
      text-align: center
    }
    .nav-label: {
      overflow: hidden
      opacity: 1
      transition: opacity 0.15s
    }
    .nav-label.hidden: { opacity: 0; width: 0; overflow: hidden }
    .toggle-btn: {
      pad: 16px 24px
      border-top: 1px solid var(--border)
      cursor: pointer
      color: var(--text-muted)
      display: flex
      align-items: center
      gap: 14px
      transition: color 0.2s
    }
    .toggle-btn:hover: { color: var(--text-primary) }
    .shop-badge: {
      margin: 0 16px 16px
      pad: 12px 14px
      radius: 10px
      bg: linear-gradient(135deg, rgba(212,168,83,0.15), rgba(196,92,122,0.15))
      border: 1px solid rgba(212,168,83,0.25)
      display: flex
      align-items: center
      gap: 10px
    }
    .badge-dot: {
      width: 8px
      height: 8px
      radius: 50%
      bg: var(--success)
      flex-shrink: 0
      box-shadow: 0 0 6px var(--success)
      animate: pulse 2s ease infinite
    }
    .badge-text: {
      font: var(--font-body) 12px 500
      color: var(--accent-gold)
      white-space: nowrap
      overflow: hidden
    }
  }

  render {
    box.root {
      // Logo
      box.logo-zone {
        text.logo-icon { "🌸" }
        text.logo-text class:{ collapsed ? "hidden" : "" } {
          "Gullar Bog'i"
        }
      }

      // Navigation items
      box.nav-section {
        each item in ~navItems {
          box.nav-item
            class:{ active == item.id ? "active" : "" }
            on:click={ emit("navigate", item.id) }
          {
            text.nav-icon { item.icon }
            text.nav-label class:{ collapsed ? "hidden" : "" } {
              item.label
            }
          }
        }
      }

      // Online status badge
      if collapsed.not {
        box.shop-badge {
          box.badge-dot {}
          text.badge-text { "Do'kon ochiq • 08:00–20:00" }
        }
      }

      // Collapse toggle
      box.toggle-btn on:click={ emit("toggle") } {
        text style:"font-size:16px" { collapsed ? "»" : "«" }
        if collapsed.not {
          text style:"font-size:13px" { "Yig'ish" }
        }
      }
    }
  }
}


// ─── DASHBOARD PAGE ─────────────────────────────────────────
bloom DashboardPage {
  ~todayRevenue => AppData.orders
    .filter(o => o.created_at.startsWith("2026-06-07"))
    .sum(o => o.total)

  ~todayOrders => AppData.orders
    .filter(o => o.created_at.startsWith("2026-06-07"))
    .count

  ~pendingCount => AppData.orders
    .filter(o => o.status == "pending")
    .count

  ~topProduct => AppData.products
    .sortBy(p => p.sold, "desc")
    .first

  ~chartMax => AppData.dailyStats.max(s => s.revenue)

  style {
    root: {
      pad: 32px 36px
      min-height: 100%
    }
    .page-header: {
      margin-bottom: 32px
    }
    .page-title: {
      font: var(--font-display) 36px 600
      color: var(--text-primary)
      letter-spacing: -0.5px
    }
    .page-subtitle: {
      font: var(--font-body) 14px
      color: var(--text-secondary)
      margin-top: 4px
    }
    .kpi-grid: {
      display: grid
      grid: auto / repeat(4, 1fr)
      gap: 20px
      margin-bottom: 32px
    }
    .kpi-card: {
      bg: var(--bg-card)
      border: 1px solid var(--border)
      radius: 16px
      pad: 22px 24px
      position: relative
      overflow: hidden
      transition: transform 0.2s, border-color 0.2s
    }
    .kpi-card:hover: {
      transform: translateY(-2px)
      border-color: var(--accent-gold)
    }
    .kpi-card::before: {
      content: ""
      position: absolute
      top: 0; right: 0
      width: 80px; height: 80px
      radius: 50%
      blur: 40px
      opacity: 0.15
    }
    .kpi-card.gold::before:  { bg: var(--accent-gold) }
    .kpi-card.rose::before:  { bg: var(--accent-rose) }
    .kpi-card.sage::before:  { bg: var(--accent-sage) }
    .kpi-card.lav::before:   { bg: var(--accent-lavender) }
    .kpi-label: {
      font: var(--font-body) 12px 500
      color: var(--text-secondary)
      text-transform: uppercase
      letter-spacing: 1px
      margin-bottom: 10px
    }
    .kpi-value: {
      font: var(--font-display) 32px 600
      color: var(--text-primary)
      line-height: 1
      margin-bottom: 6px
    }
    .kpi-change: {
      font: var(--font-body) 12px
      color: var(--success)
    }
    .kpi-icon: {
      position: absolute
      top: 20px; right: 20px
      font-size: 28px
      opacity: 0.6
    }
    .bottom-grid: {
      display: grid
      grid: auto / 2fr 1fr
      gap: 20px
    }
    .card: {
      bg: var(--bg-card)
      border: 1px solid var(--border)
      radius: 16px
      overflow: hidden
    }
    .card-header: {
      pad: 20px 24px 16px
      border-bottom: 1px solid var(--border)
      display: flex
      align-items: center
      justify-content: space-between
    }
    .card-title: {
      font: var(--font-display) 18px 600
      color: var(--text-primary)
    }
    .card-body: {
      pad: 20px 24px
    }
    .chart-area: {
      display: flex
      align-items: flex-end
      gap: 8px
      height: 160px
    }
    .chart-col: {
      flex: 1
      display: flex
      flex-direction: column
      align-items: center
      gap: 6px
      height: 100%
      justify-content: flex-end
    }
    .chart-bar-wrap: {
      width: 100%
      display: flex
      align-items: flex-end
      justify-content: center
      flex: 1
    }
    .chart-bar: {
      width: 100%
      max-width: 28px
      radius: 4px 4px 0 0
      bg: linear-gradient(180deg, var(--accent-gold), rgba(212,168,83,0.4))
      transition: opacity 0.2s
      animate: growUp 0.6s ease backwards
    }
    .chart-bar:hover: { opacity: 0.75 }
    .chart-label: {
      font: var(--font-mono) 10px
      color: var(--text-muted)
      white-space: nowrap
    }
    .top-list: {
      display: flex
      flex-direction: column
      gap: 14px
    }
    .top-item: {
      display: flex
      align-items: center
      gap: 14px
    }
    .top-rank: {
      width: 26px
      height: 26px
      radius: 50%
      bg: var(--bg-hover)
      display: flex
      align-items: center
      justify-content: center
      font: var(--font-mono) 11px 700
      color: var(--accent-gold)
      flex-shrink: 0
    }
    .top-emoji: {
      font-size: 22px
      flex-shrink: 0
    }
    .top-info: { flex: 1 }
    .top-name: {
      font: var(--font-body) 13px 600
      color: var(--text-primary)
    }
    .top-sold: {
      font: var(--font-body) 12px
      color: var(--text-secondary)
    }
    .top-bar: {
      height: 4px
      radius: 2px
      bg: var(--bg-hover)
      margin-top: 4px
    }
    .top-bar-fill: {
      height: 100%
      radius: 2px
      bg: linear-gradient(90deg, var(--accent-gold), var(--accent-rose))
    }
    .recent-list: {
      display: flex
      flex-direction: column
      gap: 0
    }
    .recent-item: {
      display: flex
      align-items: center
      gap: 14px
      pad: 14px 0
      border-bottom: 1px solid var(--border)
    }
    .recent-item:last-child: { border-bottom: none }
    .order-id: {
      font: var(--font-mono) 12px 600
      color: var(--accent-gold)
      width: 54px
      flex-shrink: 0
    }
    .order-customer: {
      flex: 1
      font: var(--font-body) 13px
      color: var(--text-primary)
    }
    .order-amount: {
      font: var(--font-display) 16px 600
      color: var(--text-primary)
      white-space: nowrap
    }
    .status-badge: {
      pad: 3px 10px
      radius: 20px
      font: var(--font-body) 11px 600
      text-transform: uppercase
      letter-spacing: 0.5px
    }
    .status-badge.pending:    { bg: rgba(224,170,92,0.15); color: var(--warning) }
    .status-badge.processing: { bg: rgba(139,127,199,0.15); color: var(--accent-lavender) }
    .status-badge.delivered:  { bg: rgba(92,174,130,0.15); color: var(--success) }
    .status-badge.cancelled:  { bg: rgba(224,92,92,0.15); color: var(--danger) }
  }

  render {
    box.root {
      // Header
      box.page-header {
        text.page-title { "Bosh sahifa" }
        text.page-subtitle { "2026-yil, 7-iyun · Shanba · Bugungi holat" }
      }

      // KPI cards
      box.kpi-grid {
        box.kpi-card.gold {
          text.kpi-icon { "💰" }
          text.kpi-label { "Bugungi daromad" }
          text.kpi-value { (~todayRevenue / 1000).toFixed(0) + " K UZS" }
          text.kpi-change { "▲ 14.2% kechagiga nisbatan" }
        }
        box.kpi-card.rose {
          text.kpi-icon { "📦" }
          text.kpi-label { "Buyurtmalar" }
          text.kpi-value { ~todayOrders.toString() }
          text.kpi-change { "▲ 6 ta yangi" }
        }
        box.kpi-card.sage {
          text.kpi-icon { "⏳" }
          text.kpi-label { "Kutilayotgan" }
          text.kpi-value { ~pendingCount.toString() }
          text.kpi-change style:"color:var(--warning)" { "● Jarayonda" }
        }
        box.kpi-card.lav {
          text.kpi-icon { "🌹" }
          text.kpi-label { "Top mahsulot" }
          text.kpi-value style:"font-size:22px" { ~topProduct.name }
          text.kpi-change { ~topProduct.sold + " ta sotilgan" }
        }
      }

      // Bottom grid
      box.bottom-grid {
        // Chart + recent orders
        box {
          display: flex
          flex-direction: column
          gap: 20px
        } {
          // Hourly chart
          box.card {
            box.card-header {
              text.card-title { "Soatlik savdo grafigi" }
              text style:"font:var(--font-body) 12px; color:var(--text-secondary)" {
                "Bugun, " + AppData.dailyStats.count + " soat"
              }
            }
            box.card-body {
              box.chart-area {
                each stat in AppData.dailyStats {
                  box.chart-col {
                    box.chart-bar-wrap {
                      box.chart-bar style:{
                        "height:" + (stat.revenue / ~chartMax * 100).toFixed(0) + "%"
                      } {}
                    }
                    text.chart-label { stat.hour }
                  }
                }
              }
            }
          }

          // Recent orders
          box.card {
            box.card-header {
              text.card-title { "So'nggi buyurtmalar" }
            }
            box.card-body {
              box.recent-list {
                each order in AppData.orders.take(5) {
                  box.recent-item {
                    text.order-id { "#" + order.id }
                    text.order-customer { order.customer }
                    text.order-amount { (order.total / 1000).toFixed(0) + "K" }
                    box.status-badge class:order.status {
                      text { order.status }
                    }
                  }
                }
              }
            }
          }
        }

        // Top products sidebar
        box.card {
          box.card-header {
            text.card-title { "Top mahsulotlar" }
          }
          box.card-body {
            box.top-list {
              each product in AppData.products.sortBy(p=>p.sold,"desc").take(6) {
                ~idx => AppData.products.sortBy(p=>p.sold,"desc").indexOf(product) + 1
                box.top-item {
                  box.top-rank { text { ~idx } }
                  text.top-emoji { product.image }
                  box.top-info {
                    text.top-name { product.name }
                    text.top-sold { product.sold + " ta · " + (product.price/1000).toFixed(1) + "K UZS" }
                    box.top-bar {
                      box.top-bar-fill style:{
                        "width:" + (product.sold / ~topProduct.sold * 100).toFixed(0) + "%"
                      } {}
                    }
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}


// ─── PRODUCTS PAGE ──────────────────────────────────────────
bloom ProductsPage {
  ~products: arr[Product] = AppData.products.clone()
  ~search: str = ""
  ~filterCategory: str = "Barchasi"
  ~showAddForm: bool = false
  ~editingId: int = -1

  // Add form state
  ~newName:     str   = ""
  ~newCategory: str   = "Atirgul"
  ~newPrice:    float = 0
  ~newStock:    int   = 0
  ~newImage:    str   = "🌸"

  ~categories => ["Barchasi"] + AppData.products.map(p=>p.category).unique()

  ~filtered => ~products
    .filter(p => ~search == "" || p.name.lower().contains(~search.lower()))
    .filter(p => ~filterCategory == "Barchasi" || p.category == ~filterCategory)

  fn addProduct() {
    ~products.push({
      id: ~products.maxId() + 1,
      name: ~newName,
      category: ~newCategory,
      price: ~newPrice,
      stock: ~newStock,
      image: ~newImage,
      sold: 0
    })
    AppData.products = ~products
    ~showAddForm = false
    ~newName = ""; ~newPrice = 0; ~newStock = 0
    emit("notify", { msg: "Mahsulot qo'shildi!", type: "success" })
  }

  fn deleteProduct(id: int) {
    ~products = ~products.filter(p => p.id != id)
    AppData.products = ~products
    emit("notify", { msg: "Mahsulot o'chirildi", type: "info" })
  }

  fn saveEdit(id: int, field: str, value: any) {
    ~products = ~products.map(p => p.id == id ? p.set(field, value) : p)
    AppData.products = ~products
  }

  style {
    root: { pad: 32px 36px }
    .toolbar: {
      display: flex
      align-items: center
      gap: 14px
      margin-bottom: 28px
      flex-wrap: wrap
    }
    .page-title: {
      font: var(--font-display) 36px 600
      color: var(--text-primary)
      margin-bottom: 24px
    }
    .search-box: {
      flex: 1
      min-width: 200px
      bg: var(--bg-card)
      border: 1px solid var(--border)
      radius: 10px
      pad: 10px 16px
      color: var(--text-primary)
      font: var(--font-body) 14px
      outline: none
      transition: border-color 0.2s
    }
    .search-box:focus: { border-color: var(--accent-gold) }
    .filter-btn: {
      pad: 9px 16px
      radius: 10px
      bg: var(--bg-card)
      border: 1px solid var(--border)
      color: var(--text-secondary)
      cursor: pointer
      font: var(--font-body) 13px 500
      transition: all 0.2s
    }
    .filter-btn.active: {
      bg: rgba(212,168,83,0.15)
      border-color: var(--accent-gold)
      color: var(--accent-gold)
    }
    .add-btn: {
      pad: 10px 20px
      radius: 10px
      bg: var(--accent-gold)
      color: #0d0d14
      border: none
      cursor: pointer
      font: var(--font-body) 14px 600
      transition: opacity 0.2s
      white-space: nowrap
    }
    .add-btn:hover: { opacity: 0.85 }
    .product-grid: {
      display: grid
      grid: auto / repeat(auto-fill, minmax(240px, 1fr))
      gap: 20px
    }
    .product-card: {
      bg: var(--bg-card)
      border: 1px solid var(--border)
      radius: 16px
      overflow: hidden
      transition: transform 0.2s, border-color 0.2s
      position: relative
    }
    .product-card:hover: {
      transform: translateY(-3px)
      border-color: var(--accent-gold)
    }
    .product-card.editing: {
      border-color: var(--accent-lavender)
    }
    .product-img: {
      height: 110px
      display: flex
      align-items: center
      justify-content: center
      font-size: 54px
      bg: linear-gradient(135deg, rgba(212,168,83,0.08), rgba(196,92,122,0.08))
      position: relative
    }
    .stock-badge: {
      position: absolute
      top: 10px; right: 10px
      pad: 3px 8px
      radius: 6px
      font: var(--font-mono) 11px 700
    }
    .stock-badge.low:  { bg: rgba(224,92,92,0.2); color: var(--danger) }
    .stock-badge.ok:   { bg: rgba(92,174,130,0.2); color: var(--success) }
    .product-info: { pad: 16px }
    .product-name: {
      font: var(--font-display) 17px 600
      color: var(--text-primary)
      margin-bottom: 4px
    }
    .product-cat: {
      font: var(--font-body) 12px
      color: var(--accent-gold)
      margin-bottom: 10px
    }
    .product-price-row: {
      display: flex
      align-items: center
      justify-content: space-between
      margin-bottom: 12px
    }
    .product-price: {
      font: var(--font-display) 20px 600
      color: var(--text-primary)
    }
    .product-sold: {
      font: var(--font-body) 12px
      color: var(--text-secondary)
    }
    .product-actions: {
      display: flex
      gap: 8px
    }
    .btn-edit: {
      flex: 1
      pad: 8px
      radius: 8px
      bg: rgba(139,127,199,0.1)
      border: 1px solid rgba(139,127,199,0.3)
      color: var(--accent-lavender)
      cursor: pointer
      font: var(--font-body) 12px 500
      transition: all 0.2s
    }
    .btn-edit:hover: { bg: rgba(139,127,199,0.2) }
    .btn-del: {
      flex: 1
      pad: 8px
      radius: 8px
      bg: rgba(224,92,92,0.08)
      border: 1px solid rgba(224,92,92,0.25)
      color: var(--danger)
      cursor: pointer
      font: var(--font-body) 12px 500
      transition: all 0.2s
    }
    .btn-del:hover: { bg: rgba(224,92,92,0.18) }
    .edit-field: {
      width: 100%
      bg: var(--bg-hover)
      border: 1px solid var(--border)
      radius: 6px
      pad: 6px 10px
      color: var(--text-primary)
      font: var(--font-body) 13px
      margin-bottom: 8px
      outline: none
    }
    .edit-field:focus: { border-color: var(--accent-lavender) }

    // Add form overlay
    .overlay: {
      position: fixed
      inset: 0
      bg: rgba(0,0,0,0.7)
      z-index: 1000
      display: flex
      align-items: center
      justify-content: center
      backdrop-filter: blur(4px)
    }
    .form-card: {
      bg: var(--bg-card)
      border: 1px solid var(--border)
      radius: 20px
      pad: 32px
      width: 420px
      max-width: 90vw
      animate: fadeScaleIn 0.25s ease
    }
    .form-title: {
      font: var(--font-display) 26px 600
      color: var(--text-primary)
      margin-bottom: 24px
    }
    .form-label: {
      display: block
      font: var(--font-body) 12px 500
      color: var(--text-secondary)
      text-transform: uppercase
      letter-spacing: 0.8px
      margin-bottom: 6px
    }
    .form-input: {
      width: 100%
      bg: var(--bg-hover)
      border: 1px solid var(--border)
      radius: 10px
      pad: 11px 14px
      color: var(--text-primary)
      font: var(--font-body) 14px
      margin-bottom: 16px
      outline: none
      box-sizing: border-box
      transition: border-color 0.2s
    }
    .form-input:focus: { border-color: var(--accent-gold) }
    .form-row: {
      display: grid
      grid: auto / 1fr 1fr
      gap: 12px
    }
    .form-footer: {
      display: flex
      gap: 12px
      margin-top: 8px
    }
    .btn-cancel: {
      flex: 1; pad: 12px
      radius: 10px
      bg: var(--bg-hover)
      border: 1px solid var(--border)
      color: var(--text-secondary)
      cursor: pointer
      font: var(--font-body) 14px 500
    }
    .btn-save: {
      flex: 2; pad: 12px
      radius: 10px
      bg: var(--accent-gold)
      border: none
      color: #0d0d14
      cursor: pointer
      font: var(--font-body) 14px 700
    }
    .emoji-picker: {
      display: flex
      gap: 10px
      flex-wrap: wrap
      margin-bottom: 16px
    }
    .emoji-opt: {
      font-size: 26px
      cursor: pointer
      pad: 6px
      radius: 8px
      transition: bg 0.15s
    }
    .emoji-opt:hover: { bg: var(--bg-hover) }
    .emoji-opt.selected: { bg: rgba(212,168,83,0.2) }
  }

  render {
    box.root {
      text.page-title { "Mahsulotlar" }

      // Toolbar
      box.toolbar {
        input.search-box
          bind:~search
          placeholder:"Gul nomini qidiring..."
        {}

        each cat in ~categories {
          box.filter-btn
            class:{ ~filterCategory == cat ? "active" : "" }
            on:click={ ~filterCategory = cat }
          {
            text { cat }
          }
        }

        box.add-btn on:click={ ~showAddForm = true } {
          text { "＋ Yangi mahsulot" }
        }
      }

      // Product grid
      box.product-grid {
        each product in ~filtered {
          box.product-card class:{ ~editingId == product.id ? "editing" : "" } {
            box.product-img {
              text { product.image }
              box.stock-badge class:{ product.stock < 30 ? "low" : "ok" } {
                text { product.stock + " dona" }
              }
            }
            box.product-info {
              if ~editingId == product.id {
                // Inline edit mode
                input.edit-field
                  value=product.name
                  on:input={ saveEdit(product.id, "name", $event.value) }
                  placeholder:"Nomi"
                {}
                input.edit-field
                  type="number"
                  value=product.price
                  on:input={ saveEdit(product.id, "price", $event.value.toFloat()) }
                  placeholder:"Narx (UZS)"
                {}
                input.edit-field
                  type="number"
                  value=product.stock
                  on:input={ saveEdit(product.id, "stock", $event.value.toInt()) }
                  placeholder:"Ombor"
                {}
              } else {
                text.product-name { product.name }
                text.product-cat  { product.category }
                box.product-price-row {
                  text.product-price { (product.price/1000).toFixed(1) + "K UZS" }
                  text.product-sold  { product.sold + " sotildi" }
                }
              }

              box.product-actions {
                box.btn-edit on:click={
                  ~editingId = ~editingId == product.id ? -1 : product.id
                } {
                  text { ~editingId == product.id ? "✓ Saqlash" : "✎ Tahrir" }
                }
                box.btn-del on:click={ deleteProduct(product.id) } {
                  text { "✕ O'chir" }
                }
              }
            }
          }
        }
      }

      // Add product modal
      if ~showAddForm {
        box.overlay on:click={ ~showAddForm = false } {
          box.form-card on:click.stop {} {
            text.form-title { "Yangi mahsulot qo'shish" }

            label.form-label { "Emoji" }
            box.emoji-picker {
              each em in ["🌹","🌷","🌼","🌸","💐","💜","🌺","🌻","🪷","🏵️"] {
                box.emoji-opt
                  class:{ ~newImage == em ? "selected" : "" }
                  on:click={ ~newImage = em }
                {
                  text { em }
                }
              }
            }

            label.form-label { "Nomi" }
            input.form-input bind:~newName placeholder:"Masalan: Qizil atirgul" {}

            label.form-label { "Kategoriya" }
            select.form-input bind:~newCategory {
              each cat in ["Atirgul","Lola","Orkide","Lavanda","Dala guli","Peony","Suv guli"] {
                option value=cat { text { cat } }
              }
            }

            box.form-row {
              box {
                label.form-label { "Narx (UZS)" }
                input.form-input type="number" bind:~newPrice placeholder:"12000" {}
              }
              box {
                label.form-label { "Ombor (dona)" }
                input.form-input type="number" bind:~newStock placeholder:"50" {}
              }
            }

            box.form-footer {
              box.btn-cancel on:click={ ~showAddForm = false } {
                text { "Bekor" }
              }
              box.btn-save on:click={ addProduct() } {
                text { "✓ Qo'shish" }
              }
            }
          }
        }
      }
    }
  }
}


// ─── ORDERS PAGE ────────────────────────────────────────────
bloom OrdersPage {
  ~orders: arr[Order] = AppData.orders.clone()
  ~filterStatus: str = "Barchasi"
  ~selectedOrder: Order? = null
  ~showDetail: bool = false

  ~filtered => ~orders
    .filter(o => ~filterStatus == "Barchasi" || o.status == ~filterStatus)
    .sortBy(o => o.created_at, "desc")

  fn changeStatus(id: int, newStatus: str) {
    ~orders = ~orders.map(o => o.id == id ? o.set("status", newStatus) : o)
    AppData.orders = ~orders
    if ~selectedOrder != null && ~selectedOrder.id == id {
      ~selectedOrder = ~selectedOrder.set("status", newStatus)
    }
    emit("notify", { msg: "Holat yangilandi: " + newStatus, type: "success" })
  }

  fn openDetail(order: Order) {
    ~selectedOrder = order
    ~showDetail = true
  }

  style {
    root: { pad: 32px 36px }
    .page-title: {
      font: var(--font-display) 36px 600
      color: var(--text-primary)
      margin-bottom: 24px
    }
    .toolbar: {
      display: flex
      gap: 10px
      margin-bottom: 24px
      flex-wrap: wrap
    }
    .filter-chip: {
      pad: 8px 18px
      radius: 20px
      bg: var(--bg-card)
      border: 1px solid var(--border)
      color: var(--text-secondary)
      cursor: pointer
      font: var(--font-body) 13px 500
      transition: all 0.2s
    }
    .filter-chip.active: {
      bg: rgba(212,168,83,0.12)
      border-color: var(--accent-gold)
      color: var(--accent-gold)
    }
    .table-wrap: {
      bg: var(--bg-card)
      border: 1px solid var(--border)
      radius: 16px
      overflow: hidden
    }
    .table: {
      width: 100%
      border-collapse: collapse
    }
    .th: {
      text-align: left
      pad: 14px 18px
      font: var(--font-body) 11px 600
      color: var(--text-muted)
      text-transform: uppercase
      letter-spacing: 0.8px
      border-bottom: 1px solid var(--border)
      bg: var(--bg-hover)
    }
    .td: {
      pad: 16px 18px
      border-bottom: 1px solid rgba(42,42,61,0.5)
      vertical-align: middle
    }
    .tr:last-child .td: { border-bottom: none }
    .tr:hover .td: { bg: rgba(255,255,255,0.02) }
    .order-id-cell: {
      font: var(--font-mono) 13px 700
      color: var(--accent-gold)
    }
    .customer-cell: {
      font: var(--font-body) 14px
      color: var(--text-primary)
    }
    .customer-phone: {
      font: var(--font-body) 12px
      color: var(--text-secondary)
      margin-top: 2px
    }
    .items-preview: {
      font: var(--font-body) 13px
      color: var(--text-secondary)
      max-width: 200px
      overflow: hidden
      text-overflow: ellipsis
      white-space: nowrap
    }
    .total-cell: {
      font: var(--font-display) 17px 600
      color: var(--text-primary)
    }
    .status-badge: {
      pad: 4px 12px
      radius: 20px
      font: var(--font-body) 11px 700
      text-transform: uppercase
      letter-spacing: 0.5px
      display: inline-block
    }
    .status-badge.pending:    { bg: rgba(224,170,92,0.15); color: var(--warning) }
    .status-badge.processing: { bg: rgba(139,127,199,0.15); color: var(--accent-lavender) }
    .status-badge.delivered:  { bg: rgba(92,174,130,0.15); color: var(--success) }
    .status-badge.cancelled:  { bg: rgba(224,92,92,0.15); color: var(--danger) }
    .time-cell: {
      font: var(--font-mono) 12px
      color: var(--text-muted)
    }
    .actions-cell: {
      display: flex
      gap: 6px
      align-items: center
    }
    .btn-view: {
      pad: 6px 12px
      radius: 6px
      bg: rgba(212,168,83,0.1)
      border: 1px solid rgba(212,168,83,0.25)
      color: var(--accent-gold)
      cursor: pointer
      font: var(--font-body) 12px
      white-space: nowrap
    }
    .status-select: {
      bg: var(--bg-hover)
      border: 1px solid var(--border)
      radius: 6px
      pad: 5px 8px
      color: var(--text-primary)
      font: var(--font-body) 12px
      cursor: pointer
      outline: none
    }

    // Detail modal
    .overlay: {
      position: fixed; inset: 0
      bg: rgba(0,0,0,0.75)
      z-index: 1000
      display: flex
      align-items: center
      justify-content: center
      backdrop-filter: blur(4px)
    }
    .detail-card: {
      bg: var(--bg-card)
      border: 1px solid var(--border)
      radius: 20px
      pad: 36px
      width: 520px
      max-width: 95vw
      max-height: 90vh
      overflow-y: auto
      animate: fadeScaleIn 0.25s ease
    }
    .detail-header: {
      display: flex
      align-items: center
      justify-content: space-between
      margin-bottom: 28px
    }
    .detail-title: {
      font: var(--font-display) 26px 600
      color: var(--text-primary)
    }
    .close-btn: {
      width: 34px; height: 34px
      radius: 50%
      bg: var(--bg-hover)
      border: 1px solid var(--border)
      display: flex; align-items: center; justify-content: center
      cursor: pointer; color: var(--text-secondary)
      font-size: 16px
      transition: all 0.2s
    }
    .close-btn:hover: { bg: var(--border); color: var(--text-primary) }
    .detail-section: { margin-bottom: 22px }
    .detail-section-title: {
      font: var(--font-body) 11px 600
      color: var(--text-muted)
      text-transform: uppercase
      letter-spacing: 1px
      margin-bottom: 12px
    }
    .detail-row: {
      display: flex
      justify-content: space-between
      pad: 8px 0
      border-bottom: 1px solid rgba(42,42,61,0.5)
      font: var(--font-body) 14px
    }
    .detail-row:last-child: { border-bottom: none }
    .detail-key: { color: var(--text-secondary) }
    .detail-val: { color: var(--text-primary); font-weight: 500 }
    .items-table: {
      width: 100%
      border-collapse: collapse
    }
    .items-th: {
      text-align: left
      pad: 8px 10px
      font: var(--font-body) 11px 600
      color: var(--text-muted)
      text-transform: uppercase
      bg: var(--bg-hover)
    }
    .items-td: {
      pad: 10px 10px
      border-bottom: 1px solid rgba(42,42,61,0.4)
      font: var(--font-body) 13px
      color: var(--text-primary)
    }
    .total-row: {
      display: flex
      justify-content: space-between
      pad: 16px 0 0
      border-top: 2px solid var(--border)
      margin-top: 4px
    }
    .total-label: {
      font: var(--font-body) 14px 600
      color: var(--text-secondary)
    }
    .total-val: {
      font: var(--font-display) 22px 700
      color: var(--accent-gold)
    }
    .status-actions: {
      display: flex
      gap: 8px
      flex-wrap: wrap
      margin-top: 16px
    }
    .status-btn: {
      pad: 9px 16px
      radius: 8px
      border: 1px solid
      cursor: pointer
      font: var(--font-body) 13px 600
      transition: all 0.2s
    }
    .status-btn.pending:    { border-color: var(--warning); color: var(--warning); bg: rgba(224,170,92,0.08) }
    .status-btn.processing: { border-color: var(--accent-lavender); color: var(--accent-lavender); bg: rgba(139,127,199,0.08) }
    .status-btn.delivered:  { border-color: var(--success); color: var(--success); bg: rgba(92,174,130,0.08) }
    .status-btn.cancelled:  { border-color: var(--danger); color: var(--danger); bg: rgba(224,92,92,0.08) }
    .status-btn.current:    { opacity: 0.4; cursor: default }
  }

  render {
    box.root {
      text.page-title { "Buyurtmalar" }

      // Status filter
      box.toolbar {
        each s in ["Barchasi","pending","processing","delivered","cancelled"] {
          box.filter-chip
            class:{ ~filterStatus == s ? "active" : "" }
            on:click={ ~filterStatus = s }
          {
            text { s == "Barchasi" ? "Barchasi" :
                   s == "pending"    ? "⏳ Kutilmoqda" :
                   s == "processing" ? "⚙ Jarayonda" :
                   s == "delivered"  ? "✓ Yetkazildi" :
                   "✕ Bekor" }
          }
        }
      }

      // Orders table
      box.table-wrap {
        table.table {
          thead {
            tr {
              th.th { text { "# ID" } }
              th.th { text { "Mijoz" } }
              th.th { text { "Mahsulotlar" } }
              th.th { text { "Jami" } }
              th.th { text { "Holat" } }
              th.th { text { "Vaqt" } }
              th.th { text { "Amal" } }
            }
          }
          tbody {
            each order in ~filtered {
              tr.tr {
                td.td {
                  text.order-id-cell { "#" + order.id }
                }
                td.td {
                  text.customer-cell { order.customer }
                  text.customer-phone { order.phone }
                }
                td.td {
                  text.items-preview {
                    order.items.map(i => i.product_name + " ×" + i.qty).join(", ")
                  }
                }
                td.td {
                  text.total-cell { (order.total/1000).toFixed(0) + "K" }
                }
                td.td {
                  box.status-badge class:order.status {
                    text { order.status }
                  }
                }
                td.td {
                  text.time-cell { order.created_at }
                }
                td.td {
                  box.actions-cell {
                    box.btn-view on:click={ openDetail(order) } {
                      text { "Ko'rish" }
                    }
                    select.status-select
                      value=order.status
                      on:change={ changeStatus(order.id, $event.value) }
                    {
                      option value="pending"    { text { "Kutilmoqda" } }
                      option value="processing" { text { "Jarayonda" } }
                      option value="delivered"  { text { "Yetkazildi" } }
                      option value="cancelled"  { text { "Bekor" } }
                    }
                  }
                }
              }
            }
          }
        }
      }

      // Order detail modal
      if ~showDetail && ~selectedOrder != null {
        box.overlay on:click={ ~showDetail = false } {
          box.detail-card on:click.stop {} {
            box.detail-header {
              text.detail-title { "Buyurtma #" + ~selectedOrder.id }
              box.close-btn on:click={ ~showDetail = false } { text { "✕" } }
            }

            // Customer info
            box.detail-section {
              text.detail-section-title { "Mijoz ma'lumotlari" }
              box {
                box.detail-row {
                  text.detail-key { "Ism" }
                  text.detail-val { ~selectedOrder.customer }
                }
                box.detail-row {
                  text.detail-key { "Telefon" }
                  text.detail-val { ~selectedOrder.phone }
                }
                box.detail-row {
                  text.detail-key { "Manzil" }
                  text.detail-val { ~selectedOrder.address }
                }
                box.detail-row {
                  text.detail-key { "Vaqt" }
                  text.detail-val { ~selectedOrder.created_at }
                }
              }
            }

            // Items
            box.detail-section {
              text.detail-section-title { "Buyurtma tarkibi" }
              table.items-table {
                thead {
                  tr {
                    th.items-th { text { "Mahsulot" } }
                    th.items-th { text { "Soni" } }
                    th.items-th { text { "Narx" } }
                    th.items-th { text { "Jami" } }
                  }
                }
                tbody {
                  each item in ~selectedOrder.items {
                    tr {
                      td.items-td { text { item.product_name } }
                      td.items-td { text { item.qty + " ta" } }
                      td.items-td { text { (item.price/1000).toFixed(1) + "K" } }
                      td.items-td { text { (item.qty * item.price / 1000).toFixed(0) + "K" } }
                    }
                  }
                }
              }
              box.total-row {
                text.total-label { "Jami to'lov:" }
                text.total-val { (~selectedOrder.total/1000).toFixed(0) + "K UZS" }
              }
            }

            // Status change
            box.detail-section {
              text.detail-section-title { "Holatni o'zgartirish" }
              box {
                box.status-badge class:~selectedOrder.status style:"margin-bottom:14px;display:inline-block" {
                  text { "Joriy: " + ~selectedOrder.status }
                }
              }
              box.status-actions {
                each s in ["pending","processing","delivered","cancelled"] {
                  box.status-btn
                    class:{ s + (s == ~selectedOrder.status ? " current" : "") }
                    on:click={ s != ~selectedOrder.status ? changeStatus(~selectedOrder.id, s) : null }
                  {
                    text { s == "pending"    ? "⏳ Kutilmoqda" :
                           s == "processing" ? "⚙ Jarayonda" :
                           s == "delivered"  ? "✓ Yetkazildi" :
                           "✕ Bekor qilish" }
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}


// ─── CUSTOMERS PAGE ─────────────────────────────────────────
bloom CustomersPage {
  ~customers: arr[Customer] = AppData.customers.clone()
  ~search: str = ""
  ~selectedCustomer: Customer? = null
  ~showHistory: bool = false

  ~filtered => ~customers
    .filter(c => ~search == "" || c.name.lower().contains(~search.lower()) || c.phone.contains(~search))
    .sortBy(c => c.total_spent, "desc")

  fn getCustomerOrders(customerId: int) => AppData.orders
    .filter(o => o.customer == AppData.customers.find(c => c.id == customerId).name)

  style {
    root: { pad: 32px 36px }
    .page-title: {
      font: var(--font-display) 36px 600
      color: var(--text-primary)
      margin-bottom: 24px
    }
    .toolbar: {
      display: flex
      gap: 14px
      margin-bottom: 24px
    }
    .search-box: {
      flex: 1
      max-width: 400px
      bg: var(--bg-card)
      border: 1px solid var(--border)
      radius: 10px
      pad: 10px 16px
      color: var(--text-primary)
      font: var(--font-body) 14px
      outline: none
    }
    .search-box:focus: { border-color: var(--accent-gold) }
    .stats-row: {
      display: grid
      grid: auto / repeat(3, 1fr)
      gap: 16px
      margin-bottom: 28px
    }
    .mini-stat: {
      bg: var(--bg-card)
      border: 1px solid var(--border)
      radius: 12px
      pad: 18px 20px
      display: flex
      align-items: center
      gap: 16px
    }
    .mini-icon: {
      font-size: 28px
      width: 48px; height: 48px
      radius: 12px
      bg: rgba(212,168,83,0.1)
      display: flex; align-items: center; justify-content: center
    }
    .mini-info {}
    .mini-label: {
      font: var(--font-body) 12px
      color: var(--text-secondary)
    }
    .mini-value: {
      font: var(--font-display) 22px 600
      color: var(--text-primary)
    }
    .customer-grid: {
      display: grid
      grid: auto / repeat(auto-fill, minmax(300px, 1fr))
      gap: 18px
    }
    .customer-card: {
      bg: var(--bg-card)
      border: 1px solid var(--border)
      radius: 16px
      pad: 22px
      cursor: pointer
      transition: all 0.2s
      position: relative
      overflow: hidden
    }
    .customer-card:hover: {
      transform: translateY(-2px)
      border-color: rgba(212,168,83,0.4)
    }
    .customer-card::before: {
      content: ""
      position: absolute
      top: -30px; right: -30px
      width: 90px; height: 90px
      radius: 50%
      bg: rgba(212,168,83,0.06)
      blur: 20px
    }
    .c-avatar: {
      width: 48px; height: 48px
      radius: 50%
      bg: linear-gradient(135deg, var(--accent-gold), var(--accent-rose))
      display: flex; align-items: center; justify-content: center
      font: var(--font-display) 20px 600
      color: #0d0d14
      margin-bottom: 14px
      flex-shrink: 0
    }
    .c-header: {
      display: flex
      align-items: center
      gap: 14px
      margin-bottom: 14px
    }
    .c-name: {
      font: var(--font-display) 18px 600
      color: var(--text-primary)
    }
    .c-contact: {
      font: var(--font-body) 12px
      color: var(--text-secondary)
    }
    .c-stats: {
      display: grid
      grid: auto / 1fr 1fr
      gap: 10px
      margin-bottom: 14px
    }
    .c-stat-box: {
      bg: var(--bg-hover)
      radius: 8px
      pad: 10px 12px
    }
    .c-stat-label: {
      font: var(--font-body) 11px
      color: var(--text-muted)
      margin-bottom: 4px
    }
    .c-stat-val: {
      font: var(--font-display) 16px 600
      color: var(--text-primary)
    }
    .c-flower: {
      display: flex
      align-items: center
      gap: 8px
      pad: 8px 12px
      radius: 8px
      bg: rgba(212,168,83,0.07)
      border: 1px solid rgba(212,168,83,0.15)
    }
    .c-flower-label: {
      font: var(--font-body) 12px
      color: var(--text-secondary)
    }
    .c-flower-val: {
      font: var(--font-body) 13px 500
      color: var(--accent-gold)
    }
    .vip-badge: {
      position: absolute
      top: 14px; right: 14px
      pad: 3px 8px
      radius: 6px
      bg: rgba(212,168,83,0.15)
      border: 1px solid rgba(212,168,83,0.3)
      font: var(--font-mono) 10px 700
      color: var(--accent-gold)
      letter-spacing: 1px
    }
    .history-btn: {
      width: 100%
      margin-top: 14px
      pad: 9px
      radius: 8px
      bg: var(--bg-hover)
      border: 1px solid var(--border)
      color: var(--text-secondary)
      cursor: pointer
      font: var(--font-body) 13px
      transition: all 0.2s
      text-align: center
    }
    .history-btn:hover: {
      border-color: var(--accent-gold)
      color: var(--accent-gold)
    }

    // History modal
    .overlay: {
      position: fixed; inset: 0
      bg: rgba(0,0,0,0.75)
      z-index: 1000
      display: flex; align-items: center; justify-content: center
      backdrop-filter: blur(4px)
    }
    .history-card: {
      bg: var(--bg-card)
      border: 1px solid var(--border)
      radius: 20px
      pad: 32px
      width: 540px
      max-width: 95vw
      max-height: 85vh
      overflow-y: auto
      animate: fadeScaleIn 0.25s ease
    }
    .hist-header: {
      display: flex; align-items: center; justify-content: space-between
      margin-bottom: 24px
    }
    .hist-title: {
      font: var(--font-display) 24px 600
      color: var(--text-primary)
    }
    .close-btn: {
      width: 32px; height: 32px; radius: 50%
      bg: var(--bg-hover); border: 1px solid var(--border)
      display: flex; align-items: center; justify-content: center
      cursor: pointer; color: var(--text-secondary); font-size: 15px
    }
    .hist-order: {
      bg: var(--bg-hover)
      border: 1px solid var(--border)
      radius: 12px
      pad: 16px 18px
      margin-bottom: 12px
    }
    .hist-order-header: {
      display: flex; align-items: center; justify-content: space-between
      margin-bottom: 8px
    }
    .hist-order-id: {
      font: var(--font-mono) 13px 700; color: var(--accent-gold)
    }
    .hist-order-date: {
      font: var(--font-mono) 12px; color: var(--text-muted)
    }
    .hist-items: {
      font: var(--font-body) 13px; color: var(--text-secondary)
      margin-bottom: 8px
    }
    .hist-total: {
      font: var(--font-display) 17px 600; color: var(--text-primary)
    }
    .no-orders: {
      text-align: center; pad: 40px
      font: var(--font-body) 14px; color: var(--text-muted)
    }
  }

  render {
    box.root {
      text.page-title { "Mijozlar" }

      // Search
      box.toolbar {
        input.search-box
          bind:~search
          placeholder:"Ism yoki telefon bo'yicha qidiring..."
        {}
      }

      // Summary stats
      box.stats-row {
        box.mini-stat {
          box.mini-icon { text { "👥" } }
          box.mini-info {
            text.mini-label { "Jami mijozlar" }
            text.mini-value { ~customers.count.toString() }
          }
        }
        box.mini-stat {
          box.mini-icon { text { "💎" } }
          box.mini-info {
            text.mini-label { "VIP (500K+ xarid)" }
            text.mini-value { ~customers.filter(c => c.total_spent >= 500000).count.toString() }
          }
        }
        box.mini-stat {
          box.mini-icon { text { "📊" } }
          box.mini-info {
            text.mini-label { "O'rtacha xarid" }
            text.mini-value {
              (~customers.sum(c => c.total_spent) / ~customers.count / 1000).toFixed(0) + "K"
            }
          }
        }
      }

      // Customer cards
      box.customer-grid {
        each customer in ~filtered {
          box.customer-card {
            if customer.total_spent >= 500000 {
              text.vip-badge { "VIP" }
            }

            box.c-header {
              box.c-avatar {
                text { customer.name.charAt(0) }
              }
              box {
                text.c-name { customer.name }
                text.c-contact { customer.phone }
                text.c-contact { customer.email }
              }
            }

            box.c-stats {
              box.c-stat-box {
                text.c-stat-label { "Buyurtmalar" }
                text.c-stat-val { customer.total_orders + " ta" }
              }
              box.c-stat-box {
                text.c-stat-label { "Jami xarid" }
                text.c-stat-val { (customer.total_spent/1000).toFixed(0) + "K" }
              }
            }

            box.c-flower {
              text style:"font-size:18px" { "🌸" }
              text.c-flower-label { "Sevimli:" }
              text.c-flower-val { customer.favorite_flower }
            }

            box.history-btn on:click={
              ~selectedCustomer = customer
              ~showHistory = true
            } {
              text { "📋 Buyurtmalar tarixi" }
            }
          }
        }
      }

      // History modal
      if ~showHistory && ~selectedCustomer != null {
        box.overlay on:click={ ~showHistory = false } {
          box.history-card on:click.stop {} {
            box.hist-header {
              text.hist-title { ~selectedCustomer.name + " tarixi" }
              box.close-btn on:click={ ~showHistory = false } { text { "✕" } }
            }

            ~customerOrders => getCustomerOrders(~selectedCustomer.id)

            if ~customerOrders.count == 0 {
              box.no-orders { text { "Hali buyurtma yo'q" } }
            } else {
              each order in ~customerOrders {
                box.hist-order {
                  box.hist-order-header {
                    text.hist-order-id { "#" + order.id }
                    box.status-badge class:order.status style:"pad:3px 10px;radius:6px;font-size:11px" {
                      text { order.status }
                    }
                    text.hist-order-date { order.created_at }
                  }
                  text.hist-items {
                    order.items.map(i => i.product_name + " ×" + i.qty).join(" · ")
                  }
                  text.hist-total { (order.total/1000).toFixed(0) + "K UZS" }
                }
              }
            }
          }
        }
      }
    }
  }
}


// ─── SETTINGS PAGE ──────────────────────────────────────────
bloom SettingsPage {
  ~settings: obj = AppData.settings.clone()
  ~activeTab: str = "general"
  ~saved: bool = false

  fn save() {
    AppData.settings = ~settings
    emit("notify", { msg: "Sozlamalar saqlandi!", type: "success" })
    ~saved = true
    timer 2000 { ~saved = false }
  }

  style {
    root: { pad: 32px 36px; max-width: 820px }
    .page-title: {
      font: var(--font-display) 36px 600
      color: var(--text-primary)
      margin-bottom: 28px
    }
    .tabs: {
      display: flex
      gap: 4px
      margin-bottom: 28px
      bg: var(--bg-card)
      border: 1px solid var(--border)
      radius: 12px
      pad: 6px
      width: fit-content
    }
    .tab: {
      pad: 9px 20px
      radius: 8px
      cursor: pointer
      font: var(--font-body) 14px 500
      color: var(--text-secondary)
      transition: all 0.2s
    }
    .tab.active: {
      bg: rgba(212,168,83,0.15)
      color: var(--accent-gold)
    }
    .settings-card: {
      bg: var(--bg-card)
      border: 1px solid var(--border)
      radius: 16px
      overflow: hidden
    }
    .settings-section: {
      pad: 28px 32px
      border-bottom: 1px solid var(--border)
    }
    .settings-section:last-child: { border-bottom: none }
    .section-title: {
      font: var(--font-display) 20px 600
      color: var(--text-primary)
      margin-bottom: 6px
    }
    .section-desc: {
      font: var(--font-body) 13px
      color: var(--text-secondary)
      margin-bottom: 22px
    }
    .field-grid: {
      display: grid
      grid: auto / 1fr 1fr
      gap: 18px
    }
    .field-grid.single: { grid: auto / 1fr }
    .field: { display: flex; flex-direction: column; gap: 7px }
    .field-label: {
      font: var(--font-body) 12px 600
      color: var(--text-secondary)
      text-transform: uppercase
      letter-spacing: 0.8px
    }
    .field-input: {
      bg: var(--bg-hover)
      border: 1px solid var(--border)
      radius: 10px
      pad: 11px 14px
      color: var(--text-primary)
      font: var(--font-body) 14px
      outline: none
      transition: border-color 0.2s
    }
    .field-input:focus: { border-color: var(--accent-gold) }
    .days-picker: {
      display: flex
      gap: 8px
      flex-wrap: wrap
    }
    .day-chip: {
      width: 40px; height: 40px
      radius: 10px
      display: flex; align-items: center; justify-content: center
      bg: var(--bg-hover)
      border: 1px solid var(--border)
      cursor: pointer
      font: var(--font-body) 13px 600
      color: var(--text-secondary)
      transition: all 0.2s
    }
    .day-chip.active: {
      bg: rgba(212,168,83,0.15)
      border-color: var(--accent-gold)
      color: var(--accent-gold)
    }
    .color-row: {
      display: flex
      align-items: center
      gap: 16px
      margin-bottom: 12px
    }
    .color-preview: {
      width: 36px; height: 36px
      radius: 8px
      border: 2px solid rgba(255,255,255,0.1)
      flex-shrink: 0
    }
    .color-input: {
      flex: 1
      bg: var(--bg-hover)
      border: 1px solid var(--border)
      radius: 8px
      pad: 9px 12px
      color: var(--text-primary)
      font: var(--font-mono) 14px
      outline: none
    }
    .theme-swatches: {
      display: grid
      grid: auto / repeat(5, 1fr)
      gap: 10px
      margin-top: 14px
    }
    .theme-swatch: {
      height: 60px
      radius: 10px
      cursor: pointer
      border: 2px solid transparent
      transition: all 0.2s
      display: flex; align-items: center; justify-content: center
      font-size: 20px
    }
    .theme-swatch:hover: { transform: scale(1.05) }
    .theme-swatch.selected: { border-color: var(--accent-gold) }
    .footer: {
      pad: 24px 32px
      display: flex
      justify-content: flex-end
      gap: 12px
      bg: var(--bg-hover)
      border-top: 1px solid var(--border)
    }
    .btn-reset: {
      pad: 11px 24px
      radius: 10px
      bg: transparent
      border: 1px solid var(--border)
      color: var(--text-secondary)
      cursor: pointer
      font: var(--font-body) 14px 500
    }
    .btn-save: {
      pad: 11px 32px
      radius: 10px
      bg: var(--accent-gold)
      border: none
      color: #0d0d14
      cursor: pointer
      font: var(--font-body) 14px 700
      transition: opacity 0.2s
    }
    .btn-save:hover: { opacity: 0.85 }
    .btn-save.saved: { bg: var(--success) }
  }

  render {
    box.root {
      text.page-title { "Sozlamalar" }

      // Tabs
      box.tabs {
        each tab in [
          { id:"general", label:"Umumiy" },
          { id:"schedule", label:"Ish vaqti" },
          { id:"theme", label:"Mavzu" }
        ] {
          box.tab
            class:{ ~activeTab == tab.id ? "active" : "" }
            on:click={ ~activeTab = tab.id }
          {
            text { tab.label }
          }
        }
      }

      // Settings card
      box.settings-card {
        // ── General tab ──
        if ~activeTab == "general" {
          box.settings-section {
            text.section-title { "Do'kon ma'lumotlari" }
            text.section-desc { "Asosiy do'kon identifikatori va aloqa ma'lumotlari" }
            box.field-grid {
              box.field {
                text.field-label { "Do'kon nomi" }
                input.field-input
                  bind:~settings.shopName
                  placeholder:"Gullar Bog'i"
                {}
              }
              box.field {
                text.field-label { "Egasi ismi" }
                input.field-input
                  bind:~settings.ownerName
                  placeholder:"To'liq ism"
                {}
              }
              box.field {
                text.field-label { "Telefon raqami" }
                input.field-input
                  bind:~settings.phone
                  placeholder:"+998 90 000 00 00"
                {}
              }
              box.field {
                text.field-label { "Valyuta" }
                select.field-input bind:~settings.currency {
                  option value="UZS" { text { "UZS — O'zbek so'mi" } }
                  option value="USD" { text { "USD — Dollar" } }
                }
              }
            }
            box.field style:"margin-top:18px" {
              text.field-label { "Manzil" }
              input.field-input
                bind:~settings.address
                placeholder:"To'liq manzil"
              {}
            }
          }

          box.settings-section {
            text.section-title { "Soliq sozlamalari" }
            text.section-desc { "QQS va narxlash usuli" }
            box.field style:"max-width:200px" {
              text.field-label { "Soliq stavkasi (%)" }
              input.field-input
                type="number"
                bind:~settings.taxRate
                placeholder:"12"
              {}
            }
          }
        }

        // ── Schedule tab ──
        if ~activeTab == "schedule" {
          box.settings-section {
            text.section-title { "Ish vaqti" }
            text.section-desc { "Do'kon qachon ochiq bo'lishi" }
            box.field-grid {
              box.field {
                text.field-label { "Ochilish vaqti" }
                input.field-input
                  type="time"
                  bind:~settings.openTime
                {}
              }
              box.field {
                text.field-label { "Yopilish vaqti" }
                input.field-input
                  type="time"
                  bind:~settings.closeTime
                {}
              }
            }
          }

          box.settings-section {
            text.section-title { "Ish kunlari" }
            text.section-desc { "Qaysi kunlari do'kon ochiq?" }
            box.days-picker {
              each day in ["Du","Se","Ch","Pa","Ju","Sh","Ya"] {
                ~isActive => ~settings.workDays.includes(day)
                box.day-chip
                  class:{ ~isActive ? "active" : "" }
                  on:click={
                    ~isActive
                      ? ~settings.workDays = ~settings.workDays.filter(d => d != day)
                      : ~settings.workDays.push(day)
                  }
                {
                  text { day }
                }
              }
            }
          }
        }

        // ── Theme tab ──
        if ~activeTab == "theme" {
          box.settings-section {
            text.section-title { "Asosiy ranglar" }
            text.section-desc { "Do'kon ranglar sxemasi" }
            box {
              box.color-row {
                box.color-preview style:{ "background:" + ~settings.primaryColor } {}
                text style:"color:var(--text-secondary);font:var(--font-body) 13px;min-width:100px" {
                  "Asosiy rang"
                }
                input.color-input
                  bind:~settings.primaryColor
                  placeholder:"#d4a853"
                {}
              }
              box.color-row {
                box.color-preview style:{ "background:" + ~settings.accentColor } {}
                text style:"color:var(--text-secondary);font:var(--font-body) 13px;min-width:100px" {
                  "Aktsent rang"
                }
                input.color-input
                  bind:~settings.accentColor
                  placeholder:"#c45c7a"
                {}
              }
            }

            text style:"font:var(--font-body) 12px 600;color:var(--text-muted);text-transform:uppercase;letter-spacing:0.8px;margin-top:20px;display:block" {
              "Tayyor mavzular"
            }
            box.theme-swatches {
              each theme in [
                { bg:"linear-gradient(135deg,#d4a853,#c45c7a)", icon:"🌹", label:"Gulzor" },
                { bg:"linear-gradient(135deg,#6a9e7f,#3d7a5e)", icon:"🌿", label:"Yashil" },
                { bg:"linear-gradient(135deg,#8b7fc7,#5d4fa0)", icon:"💜", label:"Binafsha" },
                { bg:"linear-gradient(135deg,#5cae82,#2e8b57)", icon:"🍃", label:"Mints" },
                { bg:"linear-gradient(135deg,#e07070,#c04040)", icon:"🌺", label:"Qizil" }
              ] {
                box.theme-swatch
                  style:{ "background:" + theme.bg }
                  on:click={
                    ~settings.primaryColor = theme.bg.match(/#[0-9a-f]{6}/i)[0]
                    ~settings.accentColor  = theme.bg.match(/#[0-9a-f]{6}/i)[1]
                  }
                {
                  text { theme.icon }
                }
              }
            }
          }

          box.settings-section {
            text.section-title { "Interfeys" }
            text.section-desc { "Ko'rinish sozlamalari (kelajakda)" }
            box style:"opacity:0.4;pointer-events:none" {
              box.field-grid {
                box.field {
                  text.field-label { "Font" }
                  select.field-input {
                    option { text { "Cormorant Garamond (hozirgi)" } }
                    option { text { "Playfair Display" } }
                    option { text { "EB Garamond" } }
                  }
                }
                box.field {
                  text.field-label { "Tartib" }
                  select.field-input {
                    option { text { "Qoʻngʻir qoʻngʻir" } }
                    option { text { "Compact" } }
                  }
                }
              }
            }
          }
        }

        // Save footer
        box.footer {
          box.btn-reset on:click={ ~settings = AppData.settings.clone() } {
            text { "Bekor qilish" }
          }
          box.btn-save
            class:{ ~saved ? "saved" : "" }
            on:click={ save() }
          {
            text { ~saved ? "✓ Saqlandi!" : "Saqlash" }
          }
        }
      }
    }
  }
}


// ─── GLOBAL CSS RUNTIME (Petal compiler output) ────────────────
// Petal runtime shu CSS-ni avtomatik generate qiladi va HTML'ga inject etadi.
// Quyidagi @petal-inject bloki kompilyator ko'rsatmasi:

@petal-inject css {
  @import url('https://fonts.googleapis.com/css2?family=Cormorant+Garamond:wght@400;600;700&family=DM+Sans:wght@400;500;600&family=JetBrains+Mono:wght@400;700&display=swap');

  *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }

  :root {
    --bg-deep:         #0d0d14;
    --bg-card:         #16161f;
    --bg-hover:        #1e1e2e;
    --border:          #2a2a3d;
    --accent-gold:     #d4a853;
    --accent-rose:     #c45c7a;
    --accent-sage:     #6a9e7f;
    --accent-lavender: #8b7fc7;
    --text-primary:    #f0ece4;
    --text-secondary:  #8a8a9e;
    --text-muted:      #4a4a5e;
    --danger:          #e05c5c;
    --success:         #5cae82;
    --warning:         #e0aa5c;
    --font-display:    'Cormorant Garamond', Georgia, serif;
    --font-body:       'DM Sans', system-ui, sans-serif;
    --font-mono:       'JetBrains Mono', monospace;
  }

  body {
    background: var(--bg-deep);
    color: var(--text-primary);
    font-family: var(--font-body);
    overflow: hidden;
  }

  ::-webkit-scrollbar { width: 6px; height: 6px; }
  ::-webkit-scrollbar-track { background: transparent; }
  ::-webkit-scrollbar-thumb { background: var(--border); border-radius: 3px; }
  ::-webkit-scrollbar-thumb:hover { background: var(--text-muted); }

  @keyframes growUp {
    from { transform: scaleY(0); transform-origin: bottom; }
    to   { transform: scaleY(1); transform-origin: bottom; }
  }
  @keyframes fadeScaleIn {
    from { opacity: 0; transform: scale(0.95); }
    to   { opacity: 1; transform: scale(1); }
  }
  @keyframes slideInRight {
    from { opacity: 0; transform: translateX(40px); }
    to   { opacity: 1; transform: translateX(0); }
  }
  @keyframes spin {
    from { transform: rotate(0deg); }
    to   { transform: rotate(360deg); }
  }
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50%       { opacity: 0.4; }
  }
}

// ─── ENTRY POINT ────────────────────────────────────────────
@mount "#app" {
  render <App />
}
```

## O'z-o'zini baholash

**Kuchli tomonlari:**

1. **Ko'rinarlilik (Visual clarity)** — `~` bilan reactive state, `=>` bilan computed, `source` bilan API — har bir konstruksiya bir belgi yoki kalit so'z, xato qilish qiyin.

2. **Izolatsiya** — har `bloom` o'z `style {}` blokiga ega, global kaskad yo'q. Bu katta loyihalarda CSS konfliktini butunlay yo'q qiladi.

3. **Daraxt sintaksisi** — HTML/JSX kabi `<div>` emas, `box`, `text`, `list` kabi semantik primitivlar. Kompilyator bunlarni optimallashtirilgan DOM'ga aylantiradi.

4. **Type-first** — `type Product { ... }` bloki IDE autocomplete va runtime validatsiya uchun bir vaqtning o'zida asos bo'ladi.

5. **AI uchun qulay** — Petal'da token sarfi past: `~count += 1` bitta event handler. JSX/Vue ekvivalenti 3-4 qator. LLM uchun niyat → kod masofasi qisqa.

**Zaif tomonlari:**

1. **Runtime murakkabligi** — `~` reaktivligini, `source` HTTP so'rovlarini, `@petal-inject` CSS generatsiyasini boshqaruvchi runtime yozish juda katta ish — bu Svelte, Solid.js darajasidagi kompilyator demak.

2. **Loop ichida computed** — `~idx => ... .indexOf(product)` har render'da qayta hisoblanadi; memoizatsiya mexanizmi hali spetsifikatsiyada yo'q.

3. **Template vs Logic chegarasi** — `render {}` ichida `fn` chaqirish bilan logic aralashib ketishi mumkin; React hooks'dagi kabi qoidalar kerak bo'ladi.

4. **Tooling noldan** — LSP, formatter, debugger — barchasi yangidan yozilishi kerak.

**Nima uchun AI buni oson yozadi:** Petal'ning daraxt sintaksisi LLM'ning o'zi ishlatadigan fikrlash tarziga mos — ierarxik, qismlarni alohida yozib keyinchalik birlashtirish mumkin. `~` va `=>` kabi qisqa tokenlar attention'ni keraksiz sintaksis o'rniga mazmun (komponent nomi, state nomi) ga yo'naltiradi.