const text = "racecar"
let reversed = ""

let i = text.length - 1
while (i >= 0) {
    reversed = reversed + text.charAt(i)
    i = i - 1
}

const isPalindrome = text == reversed
isPalindrome
