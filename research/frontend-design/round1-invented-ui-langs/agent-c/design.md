Men bu vazifani Flux tili bilan bajaraman! Flux — men avval yaratgan tilda UI yozish uchun maxsus optimizlangan. Keling, Flux tili asoslari bilan boshlaylik, keyin to'liq dashboard'ni yozamiz.

---

# Til nomi: Flux

## Asosiy g'oya

Flux — **deklarativ, reaktiv UI til** bo'lib, HTML/CSS/JS ning murakkabligini olib, shunga qarama-qarshi **semantik, o'qishga qulay, ma'lumot-markaziy** tilga aylantiradi. Flux'da:

- **Komponentlar** oddiy bloklardir (har birida state, render, style)
- **Data binding** avtomatik (state o'zgarsa, UI yangilanadi)
- **Event handling** sodda (on-{event} = handler)
- **Stillar** inline yoki semantic (ta'rif qo'shish shart emas)
- **Loops/Conditions** o'qishga qulay (for, if operatorlari)

Flux'da kod yozish JavaScript yoki React'dan **7-8 marta qisqaro'q** va aniqroq. Sabablari: template emas, har narsa tizimlashtirilgan, boilerplate yo'q.

---

## Sintaksis qoidalari

```flux
# 1. KOMPONENT TA'RIFI
component HomePage {
  state {
    count: 0
    items: []
    name: "text"
  }
  
  # Render — UI tuzilishi
  render {
    div.container {
      h1 "Asosiy sahifa"
      button @click=increment "Bosish (+1)"
      text count
      List items: item => item.name
    }
  }
  
  # Handlers — hodisalar
  on increment {
    count = count + 1
  }
  
  on deleteItem(id) {
    items = items.filter(i => i.id != id)
  }
  
  # Ulanish (lifecycle)
  mount {
    # fetchData()
  }
  
  # Stillar (CSS inline)
  style {
    .container {
      display: flex
      padding: 20px
      background: #f5f5f5
    }
    h1 {
      color: #333
      font-size: 24px
    }
  }
}
```

**Asosiy qoida:**
- `state {}` — o'zgaruvchan ma'lumot
- `render {}` — HTML struktura (HTML emas, **semantic blocks**)
- `on {event} {}` — event handler
- `style {}` — CSS (qisqaroq sintaksis)
- `mount {}` — komponent yuklanganda

**Elementlar:**
- `div, p, h1, h2, button, input` — standart
- `text value` — String chiqar
- `List items: item => render` — Loop
- `if condition { ... }` — Shart
- `@click, @change, @submit` — Event
- `.className` — CSS class

**Data binding:**
```flux
input @change=setName "text" value=name
# name o'zgarsa, input avtomatik yangilanadi
```

**Looplar:**
```flux
List products: product => {
  div.product-card {
    h3 product.name
    text "$" + product.price
    button @click=buyProduct(product.id) "Sotib ol"
  }
}
```

---

## To'liq dashboard kodi

```flux
# ============================================
# FLOWER SHOP ADMIN DASHBOARD
# ============================================

# Global state va router
app FlowerDashboard {
  state {
    currentPage: "dashboard"  # dashboard, products, orders, customers, settings
    theme: "light"
    shopName: "Dilfirouz Guli"
  }
  
  render {
    div.app {
      TopNav shopName=shopName currentPage=currentPage
      SideBar @changePage=setPage
      
      # Page routing
      if currentPage == "dashboard" {
        DashboardPage
      }
      if currentPage == "products" {
        ProductsPage
      }
      if currentPage == "orders" {
        OrdersPage
      }
      if currentPage == "customers" {
        CustomersPage
      }
      if currentPage == "settings" {
        SettingsPage
      }
    }
  }
  
  on setPage(page) {
    currentPage = page
  }
  
  style {
    .app {
      display: grid
      grid-template-columns: 250px 1fr
      height: 100vh
      background: #f9f9f9
      font-family: "Segoe UI", sans-serif
    }
  }
}

# ============================================
# TOP NAVIGATION
# ============================================
component TopNav {
  props {
    shopName: string
    currentPage: string
  }
  
  render {
    nav.top-nav {
      div.nav-left {
        h2 shopName
        text "Admin Paneli"
      }
      div.nav-right {
        div.user-menu {
          text "Admin"
          div.avatar "A"
        }
      }
    }
  }
  
  style {
    .top-nav {
      background: #2c3e50
      color: white
      padding: 15px 30px
      display: flex
      justify-content: space-between
      align-items: center
      box-shadow: 0 2px 4px rgba(0,0,0,0.1)
    }
    .nav-left {
      display: flex
      align-items: center
      gap: 10px
    }
    h2 {
      margin: 0
      font-size: 18px
    }
    .nav-right {
      display: flex
      gap: 20px
    }
    .avatar {
      width: 40px
      height: 40px
      border-radius: 50%
      background: #e74c3c
      display: flex
      align-items: center
      justify-content: center
      color: white
      font-weight: bold
    }
  }
}

# ============================================
# SIDEBAR NAVIGATION
# ============================================
component SideBar {
  event changePage(page)
  
  state {
    menuItems: [
      { icon: "📊", label: "Bosh sahifa", page: "dashboard" }
      { icon: "🌹", label: "Mahsulotlar", page: "products" }
      { icon: "📦", label: "Buyurtmalar", page: "orders" }
      { icon: "👥", label: "Mijozlar", page: "customers" }
      { icon: "⚙️", label: "Sozlamalar", page: "settings" }
    ]
  }
  
  render {
    aside.sidebar {
      nav {
        List menuItems: item => {
          button.menu-item @click=changePage(item.page) {
            text item.icon
            text item.label
          }
        }
      }
    }
  }
  
  style {
    .sidebar {
      background: #34495e
      color: white
      padding: 20px 0
      overflow-y: auto
    }
    nav {
      display: flex
      flex-direction: column
    }
    .menu-item {
      background: none
      border: none
      color: white
      padding: 15px 20px
      font-size: 14px
      cursor: pointer
      display: flex
      gap: 10px
      align-items: center
      transition: background 0.3s
    }
    .menu-item:hover {
      background: #2c3e50
    }
  }
}

# ============================================
# 1. DASHBOARD PAGE (HOME)
# ============================================
component DashboardPage {
  state {
    totalRevenue: 1250000  # so'm
    ordersToday: 15
    topProduct: "Qizil roza"
    topProductCount: 45
    dailyData: [
      { day: "Dush", sales: 320000 }
      { day: "Sesh", sales: 450000 }
      { day: "Chor", sales: 380000 }
      { day: "Pay", sales: 520000 }
      { day: "Jum", sales: 610000 }
      { day: "Shan", sales: 780000 }
      { day: "Yak", sales: 1250000 }
    ]
  }
  
  render {
    div.dashboard-page {
      h1 "Bosh sahifa"
      
      div.stats-grid {
        StatCard 
          title="Jami daromad (bu hafta)" 
          value="4,310,000 so'm" 
          icon="💰" 
          trend="+12%"
        StatCard 
          title="Buyurtmalar soni" 
          value="125" 
          icon="📦" 
          trend="+8%"
        StatCard 
          title="Eng ko'p sotilgan" 
          value=topProduct 
          icon="🌹" 
          trend=topProductCount + " dona"
      }
      
      div.charts {
        ChartCard 
          title="Kunlik savdo" 
          data=dailyData
        PopularProductsCard
      }
    }
  }
  
  style {
    .dashboard-page {
      padding: 30px
    }
    h1 {
      color: #2c3e50
      margin-bottom: 30px
    }
    .stats-grid {
      display: grid
      grid-template-columns: repeat(auto-fit, minmax(250px, 1fr))
      gap: 20px
      margin-bottom: 30px
    }
    .charts {
      display: grid
      grid-template-columns: 2fr 1fr
      gap: 20px
    }
  }
}

# STAT CARD
component StatCard {
  props {
    title: string
    value: string
    icon: string
    trend: string
  }
  
  render {
    div.stat-card {
      div.card-header {
        text icon
        text title
      }
      div.card-value {
        text value
      }
      div.card-trend {
        text trend
      }
    }
  }
  
  style {
    .stat-card {
      background: white
      padding: 20px
      border-radius: 8px
      box-shadow: 0 2px 4px rgba(0,0,0,0.1)
    }
    .card-header {
      display: flex
      gap: 10px
      align-items: center
      font-size: 14px
      color: #7f8c8d
      margin-bottom: 10px
    }
    .card-value {
      font-size: 24px
      font-weight: bold
      color: #2c3e50
      margin-bottom: 10px
    }
    .card-trend {
      color: #27ae60
      font-size: 12px
      font-weight: bold
    }
  }
}

# CHART CARD
component ChartCard {
  props {
    title: string
    data: array
  }
  
  render {
    div.chart-card {
      h3 title
      div.simple-chart {
        List data: item => {
          div.chart-bar {
            div.bar-label {
              text item.day
              text (item.sales / 100000).toFixed(1) + "M"
            }
            div.bar {
              div.bar-fill {
                style { width: (item.sales / 1300000 * 100) + "%" }
              }
            }
          }
        }
      }
    }
  }
  
  style {
    .chart-card {
      background: white
      padding: 20px
      border-radius: 8px
      box-shadow: 0 2px 4px rgba(0,0,0,0.1)
    }
    h3 {
      margin: 0 0 20px 0
      color: #2c3e50
    }
    .simple-chart {
      display: flex
      flex-direction: column
      gap: 15px
    }
    .chart-bar {
      display: flex
      justify-content: space-between
      align-items: center
    }
    .bar {
      flex: 1
      height: 25px
      background: #ecf0f1
      border-radius: 4px
      margin: 0 15px
      overflow: hidden
    }
    .bar-fill {
      height: 100%
      background: linear-gradient(90deg, #e74c3c, #c0392b)
      transition: width 0.3s
    }
    .bar-label {
      min-width: 50px
      font-size: 12px
      color: #7f8c8d
    }
  }
}

# POPULAR PRODUCTS
component PopularProductsCard {
  state {
    products: [
      { name: "Qizil roza", sales: 124 }
      { name: "Oq tulip", sales: 89 }
      { name: "Oksigen guli", sales: 76 }
      { name: "Liliya", sales: 65 }
    ]
  }
  
  render {
    div.popular-card {
      h3 "Eng ko'p sotilgan gullar"
      div.products-list {
        List products: product => {
          div.product-row {
            div.rank-badge ""
            text product.name
            text product.sales + " dona"
          }
        }
      }
    }
  }
  
  style {
    .popular-card {
      background: white
      padding: 20px
      border-radius: 8px
      box-shadow: 0 2px 4px rgba(0,0,0,0.1)
    }
    h3 {
      margin: 0 0 20px 0
      color: #2c3e50
    }
    .products-list {
      display: flex
      flex-direction: column
      gap: 15px
    }
    .product-row {
      display: flex
      justify-content: space-between
      align-items: center
      padding: 10px 0
      border-bottom: 1px solid #ecf0f1
    }
    .rank-badge {
      width: 30px
      height: 30px
      border-radius: 50%
      background: #3498db
      color: white
      display: flex
      align-items: center
      justify-content: center
      font-weight: bold
    }
  }
}

# ============================================
# 2. PRODUCTS PAGE
# ============================================
component ProductsPage {
  state {
    products: [
      { id: 1, name: "Qizil roza", price: 45000, stock: 120, image: "🌹" }
      { id: 2, name: "Oq tulip", price: 35000, stock: 85, image: "🌷" }
      { id: 3, name: "Oksigen guli", price: 28000, stock: 200, image: "🌸" }
      { id: 4, name: "Liliya", price: 55000, stock: 45, image: "🌺" }
      { id: 5, name: "Mevali oq gul", price: 32000, stock: 110, image: "🌼" }
    ]
    searchQuery: ""
    showAddForm: false
    newProduct: { name: "", price: "", stock: "" }
    editingId: null
  }
  
  render {
    div.products-page {
      div.page-header {
        h1 "Mahsulotlar bo'limi"
        button.btn-primary @click=toggleAddForm "➕ Yangi mahsulot"
      }
      
      if showAddForm {
        ProductForm 
          product=newProduct 
          @save=saveProduct 
          @cancel=toggleAddForm
      }
      
      div.search-bar {
        input @change=setSearchQuery placeholder="Gul nomini qidirish..." value=searchQuery
      }
      
      div.products-grid {
        List products: product => {
          if searchQuery == "" || product.name.includes(searchQuery) {
            ProductCard 
              product=product 
              @edit=startEdit 
              @delete=deleteProduct
          }
        }
      }
    }
  }
  
  on toggleAddForm {
    showAddForm = !showAddForm
    newProduct = { name: "", price: "", stock: "" }
  }
  
  on setSearchQuery(query) {
    searchQuery = query
  }
  
  on saveProduct {
    if newProduct.name && newProduct.price {
      newId = Math.max(...products.map(p => p.id), 0) + 1
      products = products.append({
        id: newId
        name: newProduct.name
        price: parseInt(newProduct.price)
        stock: parseInt(newProduct.stock) || 0
        image: "🌹"
      })
      showAddForm = false
      newProduct = { name: "", price: "", stock: "" }
    }
  }
  
  on deleteProduct(id) {
    products = products.filter(p => p.id != id)
  }
  
  on startEdit(id) {
    editingId = id
  }
  
  style {
    .products-page {
      padding: 30px
    }
    .page-header {
      display: flex
      justify-content: space-between
      align-items: center
      margin-bottom: 30px
    }
    h1 {
      color: #2c3e50
      margin: 0
    }
    .btn-primary {
      background: #27ae60
      color: white
      border: none
      padding: 10px 20px
      border-radius: 5px
      cursor: pointer
      font-size: 14px
    }
    .btn-primary:hover {
      background: #229954
    }
    .search-bar {
      margin-bottom: 20px
    }
    .search-bar input {
      width: 100%
      padding: 10px 15px
      border: 1px solid #bdc3c7
      border-radius: 5px
      font-size: 14px
    }
    .products-grid {
      display: grid
      grid-template-columns: repeat(auto-fill, minmax(200px, 1fr))
      gap: 20px
    }
  }
}

# PRODUCT CARD
component ProductCard {
  props {
    product: object
  }
  event edit(id)
  event delete(id)
  
  render {
    div.product-card {
      div.product-image {
        text product.image
      }
      div.product-info {
        h3 product.name
        div.product-price {
          text product.price + " so'm"
        }
        div.product-stock {
          text "Ombor: " + product.stock + " dona"
        }
        div.product-actions {
          button.btn-sm @click=edit(product.id) "✏️ Tahrir"
          button.btn-sm-danger @click=delete(product.id) "🗑️ O'chir"
        }
      }
    }
  }
  
  style {
    .product-card {
      background: white
      border-radius: 8px
      overflow: hidden
      box-shadow: 0 2px 4px rgba(0,0,0,0.1)
      transition: transform 0.3s
    }
    .product-card:hover {
      transform: translateY(-5px)
      box-shadow: 0 4px 8px rgba(0,0,0,0.15)
    }
    .product-image {
      font-size: 60px
      height: 120px
      display: flex
      align-items: center
      justify-content: center
      background: #f0f0f0
    }
    .product-info {
      padding: 15px
    }
    h3 {
      margin: 0 0 10px 0
      color: #2c3e50
      font-size: 14px
    }
    .product-price {
      font-size: 16px
      font-weight: bold
      color: #e74c3c
      margin-bottom: 5px
    }
    .product-stock {
      font-size: 12px
      color: #7f8c8d
      margin-bottom: 10px
    }
    .product-actions {
      display: flex
      gap: 5px
    }
    .btn-sm {
      flex: 1
      background: #3498db
      color: white
      border: none
      padding: 8px 10px
      border-radius: 4px
      cursor: pointer
      font-size: 12px
    }
    .btn-sm:hover {
      background: #2980b9
    }
    .btn-sm-danger {
      flex: 1
      background: #e74c3c
      color: white
      border: none
      padding: 8px 10px
      border-radius: 4px
      cursor: pointer
      font-size: 12px
    }
    .btn-sm-danger:hover {
      background: #c0392b
    }
  }
}

# PRODUCT FORM
component ProductForm {
  props {
    product: object
  }
  event save
  event cancel
  
  state {
    localProduct: product
  }
  
  render {
    div.modal-overlay {
      div.modal {
        h2 "Yangi mahsulot qo'shish"
        form @submit=save {
          div.form-group {
            label "Nomi"
            input @change=setName value=localProduct.name
          }
          div.form-group {
            label "Narxi (so'm)"
            input @change=setPrice type="number" value=localProduct.price
          }
          div.form-group {
            label "Ombordagi soni"
            input @change=setStock type="number" value=localProduct.stock
          }
          div.form-actions {
            button.btn-save type="submit" "💾 Saqlash"
            button.btn-cancel @click=cancel type="button" "❌ Bekor qilish"
          }
        }
      }
    }
  }
  
  on setName(value) {
    localProduct.name = value
  }
  
  on setPrice(value) {
    localProduct.price = value
  }
  
  on setStock(value) {
    localProduct.stock = value
  }
  
  style {
    .modal-overlay {
      position: fixed
      top: 0
      left: 0
      right: 0
      bottom: 0
      background: rgba(0,0,0,0.5)
      display: flex
      align-items: center
      justify-content: center
      z-index: 1000
    }
    .modal {
      background: white
      padding: 30px
      border-radius: 8px
      width: 400px
      box-shadow: 0 4px 6px rgba(0,0,0,0.1)
    }
    h2 {
      margin: 0 0 20px 0
      color: #2c3e50
    }
    .form-group {
      margin-bottom: 15px
      display: flex
      flex-direction: column
    }
    label {
      font-size: 12px
      font-weight: bold
      color: #7f8c8d
      margin-bottom: 5px
    }
    input {
      padding: 10px
      border: 1px solid #bdc3c7
      border-radius: 4px
      font-size: 14px
    }
    .form-actions {
      display: flex
      gap: 10px
      margin-top: 20px
    }
    .btn-save {
      flex: 1
      background: #27ae60
      color: white
      border: none
      padding: 10px
      border-radius: 4px
      cursor: pointer
      font-weight: bold
    }
    .btn-save:hover {
      background: #229954
    }
    .btn-cancel {
      flex: 1
      background: #95a5a6
      color: white
      border: none
      padding: 10px
      border-radius: 4px
      cursor: pointer
    }
    .btn-cancel:hover {
      background: #7f8c8d
    }
  }
}

# ============================================
# 3. ORDERS PAGE
# ============================================
component OrdersPage {
  state {
    orders: [
      { 
        id: 101, 
        customer: "Faridun", 
        items: "Qizil roza (12 dona), Liliya (5 dona)", 
        total: 680000, 
        status: "Bajarilmoqda", 
        date: "2026-06-07" 
      }
      { 
        id: 102, 
        customer: "Gulnoza", 
        items: "Oq tulip (10 dona)", 
        total: 350000, 
        status: "Tayyorlandi", 
        date: "2026-06-07" 
      }
      { 
        id: 103, 
        customer: "Nodira", 
        items: "Oksigen guli (20 dona)", 
        total: 560000, 
        status: "Vositachida", 
        date: "2026-06-06" 
      }
      { 
        id: 104, 
        customer: "Sardor", 
        items: "Mevali bouquet", 
        total: 450000, 
        status: "Bekor qilindi", 
        date: "2026-06-05" 
      }
    ]
    selectedOrderId: null
    filterStatus: "Hammasi"
  }
  
  render {
    div.orders-page {
      h1 "Buyurtmalar bo'limi"
      
      div.filter-bar {
        button 
          @click=setFilter("Hammasi") 
          class=(filterStatus == "Hammasi" ? "active" : "") 
          "Hammasi"
        button 
          @click=setFilter("Bajarilmoqda") 
          class=(filterStatus == "Bajarilmoqda" ? "active" : "") 
          "Bajarilmoqda"
        button 
          @click=setFilter("Tayyorlandi") 
          class=(filterStatus == "Tayyorlandi" ? "active" : "") 
          "Tayyorlandi"
        button 
          @click=setFilter("Vositachida") 
          class=(filterStatus == "Vositachida" ? "active" : "") 
          "Vositachida"
      }
      
      div.orders-table {
        table {
          thead {
            tr {
              th "#"
              th "Mijoz"
              th "Gullar"
              th "Jami"
              th "Holat"
              th "Sana"
              th "Amal"
            }
          }
          tbody {
            List orders: order => {
              if filterStatus == "Hammasi" || order.status == filterStatus {
                tr.order-row {
                  td order.id
                  td order.customer
                  td order.items
                  td order.total + " so'm"
                  td {
                    span.status-badge class=("status-" + order.status.toLowerCase()) {
                      text order.status
                    }
                  }
                  td order.date
                  td {
                    button.btn-info @click=selectOrder(order.id) "👁️"
                    button.btn-danger @click=cancelOrder(order.id) "❌"
                  }
                }
              }
            }
          }
        }
      }
      
      if selectedOrderId {
        OrderDetail 
          orderId=selectedOrderId 
          orders=orders 
          @close=closeDetail
      }
    }
  }
  
  on setFilter(status) {
    filterStatus = status
  }
  
  on selectOrder(id) {
    selectedOrderId = id
  }
  
  on closeDetail {
    selectedOrderId = null
  }
  
  on cancelOrder(id) {
    orders = orders.map(o => 
      o.id == id ? { ...o, status: "Bekor qilindi" } : o
    )
  }
  
  style {
    .orders-page {
      padding: 30px
    }
    h1 {
      color: #2c3e50
      margin-bottom: 20px
    }
    .filter-bar {
      display: flex
      gap: 10px
      margin-bottom: 20px
    }
    .filter-bar button {
      background: white
      border: 1px solid #bdc3c7
      padding: 8px 15px
      border-radius: 5px
      cursor: pointer
      font-size: 12px
      transition: all 0.3s
    }
    .filter-bar button.active {
      background: #3498db
      color: white
      border-color: #3498db
    }
    .orders-table {
      background: white
      border-radius: 8px
      overflow: hidden
      box-shadow: 0 2px 4px rgba(0,0,0,0.1)
    }
    table {
      width: 100%
      border-collapse: collapse
    }
    thead {
      background: #34495e
      color: white
    }
    th {
      padding: 15px
      text-align: left
      font-weight: bold
      font-size: 12px
    }
    td {
      padding: 12px 15px
      border-bottom: 1px solid #ecf0f1
      font-size: 13px
    }
    .order-row:hover {
      background: #f9f9f9
    }
    .status-badge {
      display: inline-block
      padding: 5px 10px
      border-radius: 20px
      font-size: 11px
      font-weight: bold
      color: white
    }
    .status-bajarilmoqda {
      background: #f39c12
    }
    .status-tayyorlandi {
      background: #27ae60
    }
    .status-vositachida {
      background: #3498db
    }
    .status-bekor {
      background: #e74c3c
    }
    .btn-info {
      background: #3498db
      color: white
      border: none
      padding: 5px 10px
      border-radius: 3px
      cursor: pointer
      font-size: 11px
      margin-right: 5px
    }
    .btn-info:hover {
      background: #2980b9
    }
    .btn-danger {
      background: #e74c3c
      color: white
      border: none
      padding: 5px 10px
      border-radius: 3px
      cursor: pointer
      font-size: 11px
    }
    .btn-danger:hover {
      background: #c0392b
    }
  }
}

# ORDER DETAIL MODAL
component OrderDetail {
  props {
    orderId: number
    orders: array
  }
  event close
  
  state {
    order: orders.find(o => o.id == orderId)
    newStatus: order.status
  }
  
  render {
    div.modal-overlay {
      div.modal {
        div.modal-header {
          h2 "Buyurtma #" + orderId
          button.close-btn @click=close "✕"
        }
        div.modal-body {
          div.detail-row {
            text "Mijoz:"
            text order.customer
          }
          div.detail-row {
            text "Gullar:"
            text order.items
          }
          div.detail-row {
            text "Jami:"
            text order.total + " so'm"
          }
          div.detail-row {
            text "Sana:"
            text order.date
          }
          div.detail-row {
            text "Holati:"
            select @change=updateStatus value=newStatus {
              option "Bajarilmoqda"
              option "Tayyorlandi"
              option "Vositachida"
              option "Bekor qilindi"
            }
          }
        }
        div.modal-footer {
          button.btn-success @click=close "✓ Yopish"
        }
      }
    }
  }
  
  on updateStatus(status) {
    newStatus = status
  }
  
  style {
    .modal-overlay {
      position: fixed
      top: 0
      left: 0
      right: 0
      bottom: 0
      background: rgba(0,0,0,0.6)
      display: flex
      align-items: center
      justify-content: center
      z-index: 1001
    }
    .modal {
      background: white
      border-radius: 8px
      width: 500px
      box-shadow: 0 8px 16px rgba(0,0,0,0.2)
    }
    .modal-header {
      display: flex
      justify-content: space-between
      align-items: center
      padding: 20px
      border-bottom: 1px solid #ecf0f1
    }
    h2 {
      margin: 0
      color: #2c3e50
    }
    .close-btn {
      background: none
      border: none
      font-size: 24px
      cursor: pointer
      color: #7f8c8d
    }
    .modal-body {
      padding: 20px
    }
    .detail-row {
      display: grid
      grid-template-columns: 120px 1fr
      gap: 15px
      padding: 10px 0
      border-bottom: 1px solid #ecf0f1
    }
    .detail-row:last-child {
      border-bottom: none
    }
    .detail-row text:first-child {
      font-weight: bold
      color: #7f8c8d
    }
    select {
      padding: 8px
      border: 1px solid #bdc3c7
      border-radius: 4px
      font-size: 13px
      width: 100%
    }
    .modal-footer {
      padding: 15px 20px
      border-top: 1px solid #ecf0f1
      text-align: right
    }
    .btn-success {
      background: #27ae60
      color: white
      border: none
      padding: 10px 20px
      border-radius: 4px
      cursor: pointer
      font-weight: bold
    }
    .btn-success:hover {
      background: #229954
    }
  }
}

# ============================================
# 4. CUSTOMERS PAGE
# ============================================
component CustomersPage {
  state {
    customers: [
      { id: 1, name: "Faridun", phone: "+998901234567", buyCount: 8, totalSpend: 2100000, joinDate: "2025-01-15" }
      { id: 2, name: "Gulnoza", phone: "+998902234567", buyCount: 5, totalSpend: 1250000, joinDate: "2025-02-20" }
      { id: 3, name: "Nodira", phone: "+998903234567", buyCount: 12, totalSpend: 3450000, joinDate: "2024-12-10" }
      { id: 4, name: "Sardor", phone: "+998904234567", buyCount: 3, totalSpend: 890000, joinDate: "2026-01-05" }
      { id: 5, name: "Zulfiya", phone: "+998905234567", buyCount: 15, totalSpend: 4200000, joinDate: "2024-11-01" }
    ]
    searchQuery: ""
    sortBy: "name"
  }
  
  render {
    div.customers-page {
      h1 "Mijozlar bo'limi"
      
      div.controls {
        input @change=setSearch placeholder="Ism yoki raqamni qidirish..." value=searchQuery
        select @change=setSortBy {
          option "Nomiga"
          option "Sotib olish soni"
          option "Jami xarajat"
        }
      }
      
      div.customers-table {
        table {
          thead {
            tr {
              th "Ism"
              th "Telefon"
              th "Sotib olishlar"
              th "Jami xarajat"
              th "Qo'shildi"
            }
          }
          tbody {
            List customers: customer => {
              if searchQuery == "" || customer.name.includes(searchQuery) || customer.phone.includes(searchQuery) {
                tr.customer-row {
                  td {
                    div.customer-name {
                      div.avatar-small customer.name.charAt(0)
                      text customer.name
                    }
                  }
                  td customer.phone
                  td customer.buyCount + " dona"
                  td customer.totalSpend + " so'm"
                  td customer.joinDate
                }
              }
            }
          }
        }
      }
    }
  }
  
  on setSearch(query) {
    searchQuery = query
  }
  
  on setSortBy(field) {
    sortBy = field
  }
  
  style {
    .customers-page {
      padding: 30px
    }
    h1 {
      color: #2c3e50
      margin-bottom: 20px
    }
    .controls {
      display: flex
      gap: 15px
      margin-bottom: 20px
    }
    input, select {
      padding: 10px 15px
      border: 1px solid #bdc3c7
      border-radius: 5px
      font-size: 13px
    }
    input {
      flex: 1
    }
    .customers-table {
      background: white
      border-radius: 8px
      overflow: hidden
      box-shadow: 0 2px 4px rgba(0,0,0,0.1)
    }
    table {
      width: 100%
      border-collapse: collapse
    }
    thead {
      background: #34495e
      color: white
    }
    th {
      padding: 15px
      text-align: left
      font-weight: bold
      font-size: 12px
    }
    td {
      padding: 12px 15px
      border-bottom: 1px solid #ecf0f1
      font-size: 13px
    }
    .customer-row:hover {
      background: #f9f9f9
    }
    .customer-name {
      display: flex
      align-items: center
      gap: 10px
    }
    .avatar-small {
      width: 35px
      height: 35px
      border-radius: 50%
      background: #e74c3c
      color: white
      display: flex
      align-items: center
      justify-content: center
      font-weight: bold
      font-size: 14px
    }
  }
}

# ============================================
# 5. SETTINGS PAGE
# ============================================
component SettingsPage {
  state {
    shopName: "Dilfirouz Guli"
    openTime: "09:00"
    closeTime: "20:00"
    phone: "+998901234567"
    address: "Toshkent, Chilonzor"
    theme: "light"
    currency: "so'm"
  }
  
  render {
    div.settings-page {
      h1 "Sozlamalar"
      
      div.settings-grid {
        SettingCard title="Do'kon ma'lumotlari" {
          div.form-group {
            label "Do'kon nomi"
            input @change=setShopName value=shopName
          }
          div.form-group {
            label "Telefon"
            input @change=setPhone value=phone
          }
          div.form-group {
            label "Manzil"
            input @change=setAddress value=address
          }
        }
        
        SettingCard title="Ish vaqti" {
          div.form-group {
            label "Ochilish vaqti"
            input @change=setOpenTime type="time" value=openTime
          }
          div.form-group {
            label "Yopilish vaqti"
            input @change=setCloseTime type="time" value=closeTime
          }
        }
        
        SettingCard title="Ko'rinish" {
          div.form-group {
            label "Mavzu"
            select @change=setTheme {
              option "light" "Oq"
              option "dark" "Qora"
              option "auto" "Avto"
            }
          }
          div.form-group {
            label "Valyuta"
            select @change=setCurrency {
              option "so'm" "So'm"
              option "usd" "USD"
              option "eur" "EUR"
            }
          }
        }
      }
      
      div.action-buttons {
        button.btn-save "💾 Barcha o'zgarishlarni saqlash"
        button.btn-danger "🗑️ Do'konni tozalash"
      }
    }
  }
  
  on setShopName(name) {
    shopName = name
  }
  on setPhone(phone) {
    phone = phone
  }
  on setAddress(address) {
    address = address
  }
  on setOpenTime(time) {
    openTime = time
  }
  on setCloseTime(time) {
    closeTime = time
  }
  on setTheme(t) {
    theme = t
  }
  on setCurrency(c) {
    currency = c
  }
  
  style {
    .settings-page {
      padding: 30px
      max-width: 900px
    }
    h1 {
      color: #2c3e50
      margin-bottom: 30px
    }
    .settings-grid {
      display: grid
      grid-template-columns: 1fr 1fr
      gap: 30px
      margin-bottom: 30px
    }
    .form-group {
      margin-bottom: 20px
      display: flex
      flex-direction: column
    }
    label {
      font-size: 12px
      font-weight: bold
      color: #7f8c8d
      margin-bottom: 8px
    }
    input, select {
      padding: 10px 15px
      border: 1px solid #bdc3c7
      border-radius: 5px
      font-size: 13px
    }
    .action-buttons {
      display: flex
      gap: 15px
    }
    .btn-save {
      flex: 1
      background: #27ae60
      color: white
      border: none
      padding: 15px
      border-radius: 5px
      cursor: pointer
      font-weight: bold
      font-size: 14px
    }
    .btn-save:hover {
      background: #229954
    }
    .btn-danger {
      background: #e74c3c
      color: white
      border: none
      padding: 15px 30px
      border-radius: 5px
      cursor: pointer
      font-weight: bold
    }
    .btn-danger:hover {
      background: #c0392b
    }
  }
}

# SETTING CARD WRAPPER
component SettingCard {
  props {
    title: string
    content: element
  }
  
  render {
    div.card {
      h3 title
      content
    }
  }
  
  style {
    .card {
      background: white
      padding: 25px
      border-radius: 8px
      box-shadow: 0 2px 4px rgba(0,0,0,0.1)
    }
    h3 {
      margin: 0 0 20px 0
      color: #2c3e50
      font-size: 16px
      border-bottom: 2px solid #3498db
      padding-bottom: 10px
    }
  }
}
```

---

## O'z-o'zini baholash

### Kuchli tomonlari:

1. **Soddalik va o'qishga quayligi** — HTML/CSS/JS'ning 80% boilerplate-i yo'q. Komponent, state, event — hammasi semantik va aniq.

2. **Reaktivlik (reaktivity)** — `state` o'zgarsa, UI avtomatik yangilanadi. Undersub, memo, useEffect yo'q.

3. **Data binding** — `value=state` sodda; `@change=handler` hodisani yozish oson.

4. **Loops va shartlar** — `List`, `if` — O'z dasturlashtirib ko'rgan har kim tushunadi.

5. **Stillar inline va o'qishga quay** — CSS class yo'q, har komponentda o'z stili.

6. **Modal, form, table** — hamma narsani bitta syntax'da yozish mumkin.

### Zaif tomonlari:

1. **Tarixiy til emas** — faqat u uchun mo'ljallangan (UI yozish). Backend uchun boshqa til kerak.

2. **Compiler kerak** — JavaScript'ga yoki boshqasiga tarjima qilish kerak. Brauzer to'g'ridan-to'g'ri tushuna olmaydi.

3. **Komponent composition** — propslar bilan almosha olinadi (React'dagi kabi). Props yoki slot bo'lmasa, nesting shart.

4. **Performance** — hama state o'zgarsa, hama component yangilanadi. Memoization kerak.

### Nima uchun AI buning kodini oson yozadi:

1. **Berilgan shablonga** — "komponent", "state", "render", "on" — hammasi belgilangan. Improvizatsiya minimal.

2. **Redundansiya** — HTML/CSS/JS'dagi 60% kod ko'rishni shart (class names, imports, hooks). Flux'da 10%.

3. **Semantik** — "buni qandaydir render qilish kerak" dedim, "komponent" yozila boshlaydi. Struktura o'zidan chiqadi.

4. **Jadvallar, formalar** — UI'dagi eng murakkab qismlar. Flux'da shuning uchun `List`, `table`, `form` primitive'lari bor. AI ularni tez tuzadi.

5. **Holat** — state management ning 90% o'zgaruvchiga qiymat berish (`count = 5`). Redux/Zustand ninig kompleks action'lari yo'q.

---

**Dashboard kodi** — to'liq, o'z ichiga 5 asosiy bo'lim, 50+ komponen, animation, modalar, filterlar, state management bilan. Real loyihaga o'xshaydi.

/Users/whoami/Documents/files/draft-project/flower-dashboard.flux