text = "hello world from javascript"
words = text.split(" ")
wordCount = words.length

totalChars = 0
i = 0
while (i < words.length) {
    totalChars = totalChars + words[i].length
    i = i + 1
}

avgWordLength = Math.floor(totalChars / wordCount)
result = wordCount * 100 + avgWordLength
result
