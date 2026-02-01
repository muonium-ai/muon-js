const items = [1, 2, 3, 2, 4, 1, 5, 3]
const unique = []

let i = 0
while (i < items.length) {
    const item = items[i]
    let found = false
    let j = 0
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
