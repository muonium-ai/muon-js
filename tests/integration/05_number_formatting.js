price = 1234
tax = 0.08

total = price * (1 + tax)
rounded = Math.floor(total + 0.5)

priceStr = "" + rounded
formatted = "$" + priceStr

formatted
