data = [5, 2, 8, 1, 9, 3]

sum = 0
i = 0
while (i < data.length) {
    sum = sum + data[i]
    i = i + 1
}

max = data[0]
i = 1
while (i < data.length) {
    if (data[i] > max) {
        max = data[i]
    }
    i = i + 1
}

result = sum * 100 + max
result
