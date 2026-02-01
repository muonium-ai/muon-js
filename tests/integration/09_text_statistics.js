const text = "hello world from javascript"
const words = text.split(" ")
const wordCount = words.length

let totalChars = 0
let i = 0
while (i < words.length) {
    totalChars = totalChars + words[i].length
    i = i + 1
}

const avgWordLength = Math.floor(totalChars / wordCount)
const result = wordCount * 100 + avgWordLength
result
