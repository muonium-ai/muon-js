const name = "john doe"
const words = name.split(" ")
const firstName = words[0]
const lastName = words[1]

const capitalizedFirst = firstName.charAt(0).toUpperCase() + firstName.substring(1)
const capitalizedLast = lastName.charAt(0).toUpperCase() + lastName.substring(1)

const fullName = capitalizedFirst + " " + capitalizedLast
fullName
