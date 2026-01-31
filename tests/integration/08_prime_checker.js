function isPrime(n) {
    if (n <= 1) {
        return false
    }
    if (n == 2) {
        return true
    }
    if (n % 2 == 0) {
        return false
    }
    var i = 3
    while (i * i <= n) {
        if (n % i == 0) {
            return false
        }
        i = i + 2
    }
    return true
}

var primes = []
var num = 2
while (num <= 20) {
    if (isPrime(num)) {
        primes.push(num)
    }
    num = num + 1
}

primes.length
