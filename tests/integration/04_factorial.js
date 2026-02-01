function factorial(n) {
    if (n <= 1) {
        return 1
    }
    let result = 1
    let i = 2
    while (i <= n) {
        result = result * i
        i = i + 1
    }
    return result
}

const f5 = factorial(5)
const f6 = factorial(6)
f5 + f6
