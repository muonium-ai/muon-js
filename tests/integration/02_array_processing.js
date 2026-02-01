const data = [5, 2, 8, 1, 9, 3]

let sum = 0
let i = 0
while (i < data.length) {
    sum = sum + data[i]
    i = i + 1
}

let max = data[0]
i = 1
while (i < data.length) {
    if (data[i] > max) {
        max = data[i]
    }
    i = i + 1
}

const result = sum * 100 + max
result
