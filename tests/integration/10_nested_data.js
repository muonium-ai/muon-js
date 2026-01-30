matrix = [[1, 2, 3], [4, 5, 6], [7, 8, 9]]

sum = 0
i = 0
while (i < matrix.length) {
    row = matrix[i]
    j = 0
    while (j < row.length) {
        sum = sum + row[j]
        j = j + 1
    }
    i = i + 1
}

sum
