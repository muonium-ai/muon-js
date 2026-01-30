items = [1, 2, 3, 2, 4, 1, 5, 3]
unique = []

i = 0
while (i < items.length) {
    item = items[i]
    found = false
    j = 0
    while (j < unique.length) {
        if (unique[j] == item) {
            found = true
            break
        }
        j = j + 1
    }
    if (!found) {
        unique.push(item)
    }
    i = i + 1
}

unique.join(",")
