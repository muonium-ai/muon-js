const price = 1234
const tax = 0.08

const total = price * (1 + tax)
const rounded = Math.floor(total + 0.5)

const priceStr = "" + rounded
const formatted = "$" + priceStr

formatted
