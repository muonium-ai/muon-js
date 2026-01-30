function factorial(n) {
    if (n <= 1) {
        return 1
    }
    result = 1
    i = 2
    while (i <= n) {
        result = result * i
        i = i + 1
    }
    return result
}

f5 = factorial(5)
f6 = factorial(6)
f5 + f6
