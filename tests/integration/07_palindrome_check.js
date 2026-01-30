text = "racecar"
reversed = ""

i = text.length - 1
while (i >= 0) {
    reversed = reversed + text.charAt(i)
    i = i - 1
}

isPalindrome = text == reversed
isPalindrome
