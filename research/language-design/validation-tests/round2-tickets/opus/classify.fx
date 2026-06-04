# AI yordamchi — ticketni klassifikatsiya qilish va javob qoralash

use ai

# Ticketni klassifikatsiya qiladi: kategoriya, ustuvorlik, ishonch darajasi
# r._.conf metadata'sini natija map'iga qo'shib qaytaradi
exp fn classify subject body
  r = ai.json "Quyidagi qo'llab-quvvatlash so'rovini tasnifla. subject: ${subject}. body: ${body}" {category::other priority::medium}
  ret {category:r.category priority:r.priority conf:r._.conf}

# AI avtomatik javob qoralaydi
exp fn draft_reply subject body
  ret ai.ask "Quyidagi mijoz so'roviga professional, qisqa qo'llab-quvvatlash javobini yoz. Mavzu: ${subject}. So'rov: ${body}"
