function fib(n) {
    if (n <= 1) {
        return n
    }
    let a = 0
    let b = 1
    let i = 2
    while (i <= n) {
        let temp = a + b
        a = b
        b = temp
        i = i + 1
    }
    return b
}

const result = fib(10)
result
