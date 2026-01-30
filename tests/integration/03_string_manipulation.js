name = "john doe"
words = name.split(" ")
firstName = words[0]
lastName = words[1]

capitalizedFirst = firstName.charAt(0).toUpperCase() + firstName.substring(1)
capitalizedLast = lastName.charAt(0).toUpperCase() + lastName.substring(1)

fullName = capitalizedFirst + " " + capitalizedLast
fullName
