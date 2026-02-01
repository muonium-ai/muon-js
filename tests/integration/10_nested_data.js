const matrix = [[1, 2, 3], [4, 5, 6], [7, 8, 9]]

let sum = 0
let i = 0
while (i < matrix.length) {
    const row = matrix[i]
    let j = 0
    while (j < row.length) {
        sum = sum + row[j]
        j = j + 1
    }
    i = i + 1
}

sum
